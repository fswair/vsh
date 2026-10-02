//! Isolated contract probes. All guest files live in memory.

#[cfg(test)]
mod tests {
    use bashkit::{
        Bash, CommandResolver, DirEntry, ExecutionLimits, FileSystem, FileSystemExt, FsLimits,
        InMemoryFs, Metadata, Result, async_trait, hooks::HookAction,
    };
    use std::{
        io,
        path::{Path, PathBuf},
        sync::{Arc, Mutex},
        time::SystemTime,
    };

    #[derive(Clone, Debug)]
    struct Call {
        operation: &'static str,
        paths: Vec<PathBuf>,
    }

    struct RecordingFs {
        inner: InMemoryFs,
        calls: Mutex<Vec<Call>>,
        denied: Mutex<Vec<Call>>,
    }

    impl RecordingFs {
        fn record(&self, operation: &'static str, paths: &[&Path]) -> Result<()> {
            let call = Call {
                operation,
                paths: paths.iter().map(|path| path.to_path_buf()).collect(),
            };
            self.calls.lock().unwrap().push(call.clone());
            if paths.iter().any(|path| {
                path.starts_with("/workspace/protected")
                    || (!path.starts_with("/workspace") && **path != *Path::new("/"))
            }) {
                self.denied.lock().unwrap().push(call);
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "probe denied").into());
            }
            Ok(())
        }
    }

    #[async_trait]
    impl FileSystemExt for RecordingFs {}

    #[async_trait]
    impl FileSystem for RecordingFs {
        async fn read_file(&self, path: &Path) -> Result<Vec<u8>> {
            self.record("read", &[path])?;
            self.inner.read_file(path).await
        }

        async fn write_file(&self, path: &Path, content: &[u8]) -> Result<()> {
            self.record("write", &[path])?;
            self.inner.write_file(path, content).await
        }

        async fn append_file(&self, path: &Path, content: &[u8]) -> Result<()> {
            self.record("append", &[path])?;
            self.inner.append_file(path, content).await
        }

        async fn mkdir(&self, path: &Path, recursive: bool) -> Result<()> {
            self.record(
                if recursive {
                    "mkdir_recursive"
                } else {
                    "mkdir"
                },
                &[path],
            )?;
            self.inner.mkdir(path, recursive).await
        }

        async fn remove(&self, path: &Path, recursive: bool) -> Result<()> {
            self.record(
                if recursive {
                    "remove_recursive"
                } else {
                    "remove"
                },
                &[path],
            )?;
            self.inner.remove(path, recursive).await
        }

        async fn stat(&self, path: &Path) -> Result<Metadata> {
            self.record("stat", &[path])?;
            self.inner.stat(path).await
        }

        async fn read_dir(&self, path: &Path) -> Result<Vec<DirEntry>> {
            self.record("list", &[path])?;
            self.inner.read_dir(path).await
        }

        async fn exists(&self, path: &Path) -> Result<bool> {
            self.record("exists", &[path])?;
            self.inner.exists(path).await
        }

        async fn rename(&self, from: &Path, to: &Path) -> Result<()> {
            self.record("rename", &[from, to])?;
            self.inner.rename(from, to).await
        }

        async fn copy(&self, from: &Path, to: &Path) -> Result<()> {
            self.record("copy", &[from, to])?;
            self.inner.copy(from, to).await
        }

        async fn symlink(&self, target: &Path, link: &Path) -> Result<()> {
            self.record("symlink", &[target, link])?;
            self.inner.symlink(target, link).await
        }

        async fn read_link(&self, path: &Path) -> Result<PathBuf> {
            self.record("read_link", &[path])?;
            self.inner.read_link(path).await
        }

        async fn chmod(&self, path: &Path, mode: u32) -> Result<()> {
            self.record("chmod", &[path])?;
            self.inner.chmod(path, mode).await
        }

        async fn set_modified_time(&self, path: &Path, time: SystemTime) -> Result<()> {
            self.record("set_time", &[path])?;
            self.inner.set_modified_time(path, time).await
        }
    }

    struct UnknownCommands(Arc<Mutex<Vec<String>>>);

    impl CommandResolver for UnknownCommands {
        fn resolve(&self, name: &str) -> Option<Arc<dyn bashkit::Builtin>> {
            self.0.lock().unwrap().push(name.to_owned());
            None
        }
    }

    struct Fixture {
        bash: Bash,
        fs: Arc<RecordingFs>,
        blocked: Arc<Mutex<Vec<String>>>,
        unknown: Arc<Mutex<Vec<String>>>,
    }

    impl Fixture {
        async fn new(limits: ExecutionLimits) -> Result<Self> {
            let fs = Arc::new(RecordingFs {
                inner: InMemoryFs::new(),
                calls: Mutex::new(Vec::new()),
                denied: Mutex::new(Vec::new()),
            });
            fs.inner.mkdir(Path::new("/workspace"), false).await?;
            let blocked = Arc::new(Mutex::new(Vec::new()));
            let unknown = Arc::new(Mutex::new(Vec::new()));
            let observed = blocked.clone();
            let bash = Bash::builder()
                .fs(fs.clone())
                .cwd("/workspace")
                .limits(limits)
                .before_tool(Box::new(move |event| {
                    if event.name == "parallel" {
                        observed.lock().unwrap().push(event.name.clone());
                        HookAction::Cancel("unsupported by probe profile".to_owned())
                    } else {
                        HookAction::Continue(event)
                    }
                }))
                .command_resolver(Arc::new(UnknownCommands(unknown.clone())))
                .build();
            Ok(Self {
                bash,
                fs,
                blocked,
                unknown,
            })
        }
    }

    #[tokio::test]
    async fn dev_null_redirections_are_interpreter_operations_not_filesystem_calls() -> Result<()> {
        let mut fixture = Fixture::new(ExecutionLimits::default()).await?;
        let result = fixture
            .bash
            .exec("printf discarded > /dev/null; cat < /dev/null")
            .await?;
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.is_empty());
        assert!(fixture.fs.denied.lock().unwrap().is_empty());
        assert!(
            fixture
                .fs
                .calls
                .lock()
                .unwrap()
                .iter()
                .all(|call| { !call.paths.iter().any(|path| path == Path::new("/dev/null")) })
        );
        Ok(())
    }

    #[tokio::test]
    async fn workspace_script_dispatch_keeps_tool_guards_but_missing_scripts_skip_resolver()
    -> Result<()> {
        for invocation in ["./script", "source script", ". script"] {
            let mut fixture = Fixture::new(ExecutionLimits::default()).await?;
            fixture
                .fs
                .inner
                .write_file(
                    Path::new("/workspace/script"),
                    b"printf prefix > result; parallel true || true\n",
                )
                .await?;
            fixture
                .fs
                .inner
                .chmod(Path::new("/workspace/script"), 0o755)
                .await?;
            fixture.bash.exec(invocation).await?;
            assert_eq!(
                *fixture.blocked.lock().unwrap(),
                ["parallel"],
                "{invocation}"
            );
            assert!(fixture.unknown.lock().unwrap().is_empty(), "{invocation}");
            assert_eq!(
                fixture
                    .fs
                    .inner
                    .read_file(Path::new("/workspace/result"))
                    .await?,
                b"prefix"
            );
        }
        let mut fixture = Fixture::new(ExecutionLimits::default()).await?;
        let result = fixture.bash.exec("./missing || true").await?;
        assert_eq!(result.exit_code, 0);
        assert!(fixture.unknown.lock().unwrap().is_empty());
        assert!(fixture.fs.calls.lock().unwrap().iter().any(|call| {
            call.paths
                .iter()
                .any(|path| path == Path::new("/workspace/missing"))
        }));
        Ok(())
    }

    #[tokio::test]
    async fn before_exec_does_not_observe_nested_source_parsing() -> Result<()> {
        let count = Arc::new(Mutex::new(0));
        let observed = count.clone();
        let mut bash = Bash::builder()
            .before_exec(Box::new(move |event| {
                *observed.lock().unwrap() += 1;
                HookAction::Continue(event)
            }))
            .build();
        let result = bash
            .exec("eval 'printf eval'; sh -c 'printf nested'")
            .await?;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.as_bytes(), b"evalnested");
        assert_eq!(*count.lock().unwrap(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn regular_shell_workflows_reach_the_custom_filesystem() -> Result<()> {
        for (script, expected) in [
            (
                "printf 'alpha\nbeta\n' | grep beta > result",
                b"beta\n".as_slice(),
            ),
            (
                "printf a >> result & printf b >> result & wait",
                b"ab".as_slice(),
            ),
            (
                "printf 'a\nb\n' | xargs -P 2 -I {} sh -c 'printf {} >> result'",
                b"ab".as_slice(),
            ),
            (
                "printf 'old\n' > result; sed -i 's/old/new/' result",
                b"new\n".as_slice(),
            ),
        ] {
            let mut fixture = Fixture::new(ExecutionLimits::default()).await?;
            let result = fixture.bash.exec(script).await?;
            assert_eq!(result.exit_code, 0, "{script}: {:?}", result.stderr);
            assert_eq!(
                fixture
                    .fs
                    .inner
                    .read_file(Path::new("/workspace/result"))
                    .await?,
                expected
            );
            assert!(!fixture.fs.calls.lock().unwrap().is_empty());
            assert!(fixture.fs.denied.lock().unwrap().is_empty());
            assert!(fixture.unknown.lock().unwrap().is_empty());
        }
        Ok(())
    }

    #[tokio::test]
    async fn virtual_coprocesses_are_sequential_and_keep_nested_filesystem_authority() -> Result<()>
    {
        for script in [
            "coproc { printf a >> /workspace/order; }; printf b >> /workspace/order",
            "eval 'coproc { printf a >> /workspace/order; }'; printf b >> /workspace/order",
            "sh -c 'coproc { printf a >> /workspace/order; }'; printf b >> /workspace/order",
        ] {
            let mut fixture = Fixture::new(ExecutionLimits::default()).await?;
            let result = fixture.bash.exec(script).await?;
            assert_eq!(result.exit_code, 0);
            assert_eq!(
                fixture
                    .fs
                    .inner
                    .read_file(Path::new("/workspace/order"))
                    .await?,
                b"ab"
            );
            assert!(
                fixture
                    .fs
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|call| call.operation == "append")
                    .count()
                    >= 2
            );
        }
        let mut fixture = Fixture::new(ExecutionLimits::default()).await?;
        let result = fixture
            .bash
            .exec("eval 'coproc { cat /workspace/protected/secret || true; }'; true")
            .await?;
        assert_eq!(result.exit_code, 0);
        assert!(!fixture.fs.denied.lock().unwrap().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn nested_process_substitution_attempts_reach_the_denied_namespace() -> Result<()> {
        for script in [
            "cat <(printf content) || true",
            "eval 'cat <(printf content) || true'; true",
            "sh -c 'cat <(printf content) || true'; true",
        ] {
            let mut fixture = Fixture::new(ExecutionLimits::default()).await?;
            let _result = fixture.bash.exec(script).await;
            assert!(
                fixture
                    .fs
                    .denied
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|call| call.paths.iter().any(|path| path.starts_with("/dev/fd")))
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn failed_script_leaves_virtual_changes_but_reports_nonzero() -> Result<()> {
        let mut fixture = Fixture::new(ExecutionLimits::default()).await?;
        let result = fixture.bash.exec("printf changed > result; false").await?;
        assert_eq!(result.exit_code, 1);
        assert_eq!(
            fixture
                .fs
                .inner
                .read_file(Path::new("/workspace/result"))
                .await?,
            b"changed"
        );
        Ok(())
    }

    #[tokio::test]
    async fn caught_policy_denial_remains_observable() -> Result<()> {
        let mut fixture = Fixture::new(ExecutionLimits::default()).await?;
        let result = fixture.bash.exec("cat protected/secret || true").await?;
        assert_eq!(result.exit_code, 0);
        let denied = fixture.fs.denied.lock().unwrap();
        assert!(!denied.is_empty());
        assert!(denied.iter().all(|call| {
            call.paths
                .iter()
                .any(|path| path.starts_with("/workspace/protected"))
        }));
        Ok(())
    }

    #[tokio::test]
    async fn unsupported_dispatch_is_observable_through_nested_shell_forms() -> Result<()> {
        for script in [
            "parallel echo ::: a || true",
            "command parallel echo ::: a || true",
            "sh -c 'parallel echo ::: a || true'",
            "eval 'parallel echo ::: a || true'",
            "p() { parallel echo ::: a; }; p || true",
            "alias p=parallel\np echo ::: a || true",
        ] {
            let mut fixture = Fixture::new(ExecutionLimits::default()).await?;
            let result = fixture.bash.exec(script).await?;
            assert_eq!(result.exit_code, 0, "{script}: {:?}", result.stderr);
            let blocked = fixture.blocked.lock().unwrap();
            let unknown = fixture.unknown.lock().unwrap();
            assert!(
                !blocked.is_empty() || !unknown.is_empty(),
                "dispatch guards missed {script}"
            );
            println!("DISPATCH script={script:?} blocked={blocked:?} unknown={unknown:?}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn unresolved_commands_can_be_marked_without_resolving_host_programs() -> Result<()> {
        for script in [
            "not_a_real_command || true",
            "sh -c 'not_a_real_command || true'",
        ] {
            let mut fixture = Fixture::new(ExecutionLimits::default()).await?;
            let result = fixture.bash.exec(script).await?;
            assert_eq!(result.exit_code, 0);
            assert!(
                fixture
                    .unknown
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|name| name == "not_a_real_command")
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn atomic_sed_preserves_mode_and_exposes_temporary_operations() -> Result<()> {
        let mut fixture = Fixture::new(ExecutionLimits::default()).await?;
        let target = Path::new("/workspace/result");
        fixture.fs.inner.write_file(target, b"old\n").await?;
        fixture.fs.inner.chmod(target, 0o600).await?;
        let result = fixture.bash.exec("sed -i 's/old/new/' result").await?;
        assert_eq!(result.exit_code, 0, "{:?}", result.stderr);
        assert_eq!(fixture.fs.inner.read_file(target).await?, b"new\n");
        assert_eq!(fixture.fs.inner.stat(target).await?.mode & 0o777, 0o600);
        let calls = fixture.fs.calls.lock().unwrap();
        assert!(
            calls
                .iter()
                .any(|call| call.operation == "chmod" && call.paths[0] != target)
        );
        assert!(
            calls
                .iter()
                .any(|call| call.operation == "rename" && call.paths[1] == target)
        );
        Ok(())
    }

    #[tokio::test]
    async fn custom_fs_does_not_inherit_builder_filesystem_quotas() -> Result<()> {
        let fs = Arc::new(InMemoryFs::new());
        fs.mkdir(Path::new("/workspace"), false).await?;
        let mut bash = Bash::builder()
            .fs(fs.clone())
            .cwd("/workspace")
            .filesystem_limits(FsLimits::default().max_file_size(4))
            .build();
        let result = bash.exec("printf 123456789 > result").await?;
        assert_eq!(result.exit_code, 0);
        assert_eq!(
            fs.read_file(Path::new("/workspace/result")).await?,
            b"123456789"
        );
        Ok(())
    }

    #[tokio::test]
    async fn inspect_output_limits_across_pipelines_and_redirections() -> Result<()> {
        for script in [
            "printf abcdefghijklmnop",
            "printf abcdefghijklmnop | cat",
            "printf abcdefghijklmnop | cat > result",
            "{ printf abcdefghijklmnop; } | cat > result",
            "sh -c 'printf abcdefghijklmnop' | cat > result",
            "printf abcdefghijklmnop > result",
            "printf abcdefghijklmnop >&2",
            "printf abcdefghijklmnop >&2 2>/dev/null",
            "value=$(printf abcdefghijklmnop); printf '%s' \"$value\" > result",
            "value=$({ printf abcdefghijklmnop; }); printf '%s' \"$value\" > result",
            "value=$(sh -c 'printf abcdefghijklmnop'); printf '%s' \"$value\" > result",
        ] {
            let mut limits = ExecutionLimits::default();
            limits.max_stdout_bytes = 8;
            limits.max_stderr_bytes = 8;
            let mut fixture = Fixture::new(limits).await?;
            let result = fixture.bash.exec(script).await?;
            let file = fixture
                .fs
                .inner
                .read_file(Path::new("/workspace/result"))
                .await
                .ok();
            println!(
                "LIMIT script={script:?} exit={} stdout={} stderr={} truncated={}/{} file={:?}",
                result.exit_code,
                result.stdout.len(),
                result.stderr.len(),
                result.stdout_truncated,
                result.stderr_truncated,
                file.as_ref().map(|bytes| String::from_utf8_lossy(bytes))
            );
            assert!(result.stdout.len() <= 8);
            assert!(result.stderr.len() <= 8);
        }
        Ok(())
    }

    #[tokio::test]
    async fn before_tool_hook_preserves_binary_stdout() -> Result<()> {
        let mut fixture = Fixture::new(ExecutionLimits::default()).await?;
        fixture
            .fs
            .inner
            .write_file(Path::new("/workspace/binary"), &[0xff, 0, 0xfe])
            .await?;
        let result = fixture.bash.exec("cat binary").await?;
        assert_eq!(result.exit_code, 0);
        assert_eq!(result.stdout.as_bytes(), &[0xff, 0, 0xfe]);
        Ok(())
    }

    #[tokio::test]
    async fn after_tool_identity_hook_converts_binary_output_to_lossy_text() -> Result<()> {
        let fs = Arc::new(InMemoryFs::new());
        fs.mkdir(Path::new("/workspace"), false).await?;
        fs.write_file(Path::new("/workspace/binary"), &[0xff, 0, 0xfe])
            .await?;
        let mut bash = Bash::builder()
            .fs(fs)
            .cwd("/workspace")
            .after_tool(Box::new(HookAction::Continue))
            .build();
        let result = bash.exec("cat binary").await?;
        assert_eq!(result.exit_code, 0);
        assert_ne!(result.stdout.as_bytes(), &[0xff, 0, 0xfe]);
        assert_eq!(result.stdout.as_bytes(), "\u{fffd}\0\u{fffd}".as_bytes());
        Ok(())
    }

    #[tokio::test]
    async fn disabling_truncation_preserves_intermediate_data_under_work_limits() -> Result<()> {
        let mut limits = ExecutionLimits::default();
        limits.max_stdout_bytes = usize::MAX;
        limits.max_stderr_bytes = usize::MAX;
        limits.max_live_intermediate_bytes = 4096;
        let mut fixture = Fixture::new(limits).await?;
        let result = fixture
            .bash
            .exec("sh -c 'printf abcdefghijklmnop' | cat > result")
            .await?;
        assert_eq!(result.exit_code, 0);
        assert!(!result.stdout_truncated && !result.stderr_truncated);
        assert_eq!(
            fixture
                .fs
                .inner
                .read_file(Path::new("/workspace/result"))
                .await?,
            b"abcdefghijklmnop"
        );
        let limited = fixture
            .bash
            .exec("printf '%10000s' x | cat > oversized || true")
            .await;
        assert!(
            limited.is_err(),
            "intermediate byte limit must fail, got {limited:?}"
        );
        Ok(())
    }
}
