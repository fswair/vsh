//! Real worker tests: every guest filesystem operation is serviced by the parent.

#![cfg(all(unix, feature = "host", feature = "worker"))]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use vsh_bash::{BashCancellation, BashConfig, BashError, BashLimits, SubprocessBash};
use vsh_execution::ExecutionLimits;
use vsh_policy::{CallPolicy, PolicyDecision, PolicyInput, TransactionPolicy};
use vsh_store::BlobStore;
use vsh_types::{DiffKind, VPath};
use vsh_vfs::{BaseSnapshot, Effect, EffectOrigin, SnapshotBuilder, VirtualFs};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    directory: PathBuf,
    base: BaseSnapshot,
}
impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "vsh-bash-test-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).unwrap();
        let mut builder = SnapshotBuilder::new(BlobStore::open(&directory).unwrap());
        builder
            .add_file(path("input.txt"), b"old\nother\n", 0o600)
            .unwrap();
        builder
            .add_file(path("binary.bin"), &[0xff, 0, 0xfe], 0o600)
            .unwrap();
        builder
            .add_file(path("executable.sh"), b"old\n", 0o755)
            .unwrap();
        builder
            .add_file(path(".env"), b"mock-secret", 0o600)
            .unwrap();
        builder.add_directory(path("folder"), 0o755).unwrap();
        builder.add_file(path("folder/one"), b"one", 0o644).unwrap();
        builder.add_file(path("folder/two"), b"two", 0o644).unwrap();
        builder
            .add_symlink(path("link"), b"/etc/passwd", 0o777)
            .unwrap();
        Self {
            directory,
            base: builder.build().unwrap(),
        }
    }
    fn filesystem(&self) -> VirtualFs {
        VirtualFs::new(self.base.clone())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.directory).unwrap();
    }
}
fn path(value: &str) -> VPath {
    VPath::parse(value).unwrap()
}
fn executor() -> SubprocessBash {
    SubprocessBash::new(BashConfig::new(env!("CARGO_BIN_EXE_vsh-bash-worker"))).unwrap()
}

#[test]
fn relative_worker_selection_survives_the_child_working_directory() {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_vsh-bash-worker"));
    let current = std::env::current_dir().unwrap();
    let mut ancestor = current.as_path();
    let mut relative = PathBuf::new();
    while !executable.starts_with(ancestor) {
        ancestor = ancestor.parent().unwrap();
        relative.push("..");
    }
    relative.push(executable.strip_prefix(ancestor).unwrap());
    assert!(relative.is_relative() && relative.is_file());
    let executor = SubprocessBash::new(BashConfig::new(relative).with_worker_limits(1, 0)).unwrap();
    let fixture = Fixture::new();
    for _ in 0..2 {
        let result = executor
            .execute(
                "printf selected",
                &mut fixture.filesystem(),
                &CallPolicy::default(),
                ExecutionLimits::default(),
            )
            .unwrap();
        assert_eq!(result.stdout, b"selected");
    }
}

#[test]
fn pipelines_and_atomic_sed_use_the_parent_overlay_and_preserve_modes() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let result = executor()
        .execute(
            "cat input.txt | grep old > result.txt; sed -i 's/old/new/' input.txt executable.sh",
            &mut filesystem,
            &CallPolicy::default(),
            ExecutionLimits::default(),
        )
        .unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(filesystem.read(&path("result.txt")).unwrap(), b"old\n");
    assert_eq!(
        filesystem.read(&path("input.txt")).unwrap(),
        b"new\nother\n"
    );
    assert_eq!(
        filesystem.metadata(&path("input.txt")).unwrap().mode(),
        0o600
    );
    assert_eq!(
        filesystem.metadata(&path("executable.sh")).unwrap().mode(),
        0o755
    );
    assert!(
        filesystem
            .effects()
            .iter()
            .any(|event| event.origin == EffectOrigin::BashCall
                && matches!(event.effect, Effect::Rename { .. }))
    );
    assert!(
        filesystem
            .canonical_diff()
            .unwrap()
            .entries()
            .iter()
            .all(|entry| !entry.path.as_str().contains(".tmp"))
    );
    assert!(result.stats.os_calls > 0);
    assert!(!fixture.directory.join("input.txt").exists());
}

#[test]
fn binary_stdout_stderr_and_file_payloads_are_not_lossy_text() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let result = executor()
        .execute(
            "cat binary.bin; cat binary.bin >&2; cat binary.bin > copied.bin",
            &mut filesystem,
            &CallPolicy::default(),
            ExecutionLimits::default(),
        )
        .unwrap();
    assert_eq!(result.stdout, [0xff, 0, 0xfe]);
    assert_eq!(result.stderr, [0xff, 0, 0xfe]);
    assert_eq!(
        filesystem.read(&path("copied.bin")).unwrap(),
        [0xff, 0, 0xfe]
    );
}

#[test]
fn caught_denial_remains_in_parent_evidence_and_forces_final_policy_deny() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let policy = TransactionPolicy::default();
    let result = executor()
        .execute(
            "cat .env || true; printf safe > output.txt",
            &mut filesystem,
            policy.call_policy(),
            ExecutionLimits::default(),
        )
        .unwrap();
    assert!(!result.denied_accesses.is_empty());
    assert!(result.stats.denied_accesses > 0);
    assert!(matches!(
        policy.evaluate(PolicyInput {
            diff: &filesystem.canonical_diff().unwrap(),
            effects: filesystem.effects(),
            denied_accesses: &result.denied_accesses,
            base_node_count: 10
        }),
        PolicyDecision::Deny(_)
    ));
}

#[test]
fn ordinary_missing_file_errors_can_be_handled_without_hiding_profile_failures() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    executor()
        .execute(
            "cat missing || true; printf safe > output.txt",
            &mut filesystem,
            &CallPolicy::default(),
            ExecutionLimits::default(),
        )
        .unwrap();
    assert_eq!(filesystem.read_set()[&path("missing")].metadata, Some(None));
    assert_eq!(filesystem.read(&path("output.txt")).unwrap(), b"safe");
    for code in [
        "cat /etc/passwd || true; true",
        "eval 'cat <(printf bytes) || true'; true",
        "ln -s input.txt new-link || true; true",
        "chmod 4755 input.txt || true; true",
        "touch input.txt || true; true",
        "parallel echo ::: one two || true; true",
        "command parallel echo ::: one two || true; true",
        "df || true; true",
        "command df || true; true",
        "sh -c 'df || true'; true",
        "command vsh_unregistered_command || true; true",
        "sh -c 'parallel echo ::: one two || true'; true",
    ] {
        assert!(
            executor()
                .execute(
                    code,
                    &mut fixture.filesystem(),
                    &CallPolicy::default(),
                    ExecutionLimits::default()
                )
                .is_err(),
            "profile failure was hidden: {code}"
        );
    }
}

#[test]
fn meaningful_chmod_is_a_metadata_diff_and_mode_restoration_is_a_noop() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let executor = executor();
    executor
        .execute(
            "chmod 640 input.txt",
            &mut filesystem,
            &CallPolicy::default(),
            ExecutionLimits::default(),
        )
        .unwrap();
    let diff = filesystem.canonical_diff().unwrap();
    assert_eq!(diff.entries()[0].kind, DiffKind::MetadataChange);
    assert!(
        filesystem
            .effects()
            .iter()
            .any(|event| matches!(event.effect, Effect::ModifyMetadata { .. }))
    );
    executor
        .execute(
            "chmod 600 input.txt",
            &mut filesystem,
            &CallPolicy::default(),
            ExecutionLimits::default(),
        )
        .unwrap();
    assert!(filesystem.canonical_diff().unwrap().is_empty());
}

#[test]
fn final_failure_cannot_become_a_commit_proposal_even_with_virtual_changes() {
    let fixture = Fixture::new();
    let mut filesystem = fixture.filesystem();
    let error = executor()
        .execute(
            "printf changed > input.txt; false",
            &mut filesystem,
            &CallPolicy::default(),
            ExecutionLimits::default(),
        )
        .unwrap_err();
    assert!(matches!(error, BashError::Exit { code: 1, .. }));
    assert!(!filesystem.canonical_diff().unwrap().is_empty());
}

#[test]
fn evidence_limit_poisoning_cannot_be_caught_or_reused_after_partial_bash_writes() {
    let fixture = Fixture::new();
    let engine = executor();
    let mut filesystem = fixture.filesystem();
    let limits = ExecutionLimits {
        max_evidence_records: 8,
        ..ExecutionLimits::default()
    };
    let result = engine.execute("printf virtual > partial.txt; for i in 1 2 3 4 5 6 7 8 9 10; do test -e missing || true; done; true", &mut filesystem, &CallPolicy::default(), limits);
    assert!(matches!(
        result,
        Err(BashError::Limit(
            vsh_execution::ExecutionLimitExceeded::EvidenceRecords { .. }
        ))
    ));
    assert!(filesystem.canonical_diff().is_err());
    assert!(
        engine
            .execute(
                "true",
                &mut filesystem,
                &CallPolicy::default(),
                ExecutionLimits::default()
            )
            .is_err()
    );
    let mut fresh = fixture.filesystem();
    assert_eq!(
        engine
            .execute(
                "printf clean",
                &mut fresh,
                &CallPolicy::default(),
                ExecutionLimits::default()
            )
            .unwrap()
            .stdout,
        b"clean"
    );
}

#[test]
fn parent_limits_are_terminal_even_if_shell_would_handle_the_error() {
    let fixture = Fixture::new();
    for (code, limits) in [
        (
            "cat input.txt || true; true",
            ExecutionLimits {
                max_evidence_bytes: 0,
                ..ExecutionLimits::default()
            },
        ),
        (
            "cat .env || true; cat .env || true; true",
            ExecutionLimits {
                max_evidence_records: 1,
                ..ExecutionLimits::default()
            },
        ),
        (
            "ls folder || true; true",
            ExecutionLimits {
                max_directory_entries: 0,
                ..ExecutionLimits::default()
            },
        ),
        (
            "cat input.txt || true; true",
            ExecutionLimits {
                max_read_bytes: 0,
                ..ExecutionLimits::default()
            },
        ),
        (
            "printf changed > output.txt || true; true",
            ExecutionLimits {
                max_write_bytes: 0,
                ..ExecutionLimits::default()
            },
        ),
        (
            "cat binary.bin; cat binary.bin >&2",
            ExecutionLimits {
                max_output_bytes: 5,
                ..ExecutionLimits::default()
            },
        ),
    ] {
        assert!(
            executor()
                .execute(
                    code,
                    &mut fixture.filesystem(),
                    &CallPolicy::default(),
                    limits
                )
                .is_err(),
            "limit hidden by {code}"
        );
    }
}

#[test]
fn reset_workers_do_not_reuse_variables_cwd_functions_or_filesystem() {
    let fixture = Fixture::new();
    let executor = SubprocessBash::new(
        BashConfig::new(env!("CARGO_BIN_EXE_vsh-bash-worker")).with_worker_limits(1, 1),
    )
    .unwrap();
    executor
        .execute(
            "export LEAK=yes; cd folder; f() { printf leaked; }; printf changed > one",
            &mut fixture.filesystem(),
            &CallPolicy::default(),
            ExecutionLimits::default(),
        )
        .unwrap();
    let mut filesystem = fixture.filesystem();
    let result = executor
        .execute(
            "printf '%s|%s|%s' \"${LEAK-unset}\" \"$PWD\" \"$HOME\"; type -t f || true",
            &mut filesystem,
            &CallPolicy::default(),
            ExecutionLimits::default(),
        )
        .unwrap();
    assert_eq!(result.stdout, b"unset|/workspace|/workspace");
    assert!(filesystem.canonical_diff().unwrap().is_empty());
    assert_eq!(filesystem.read(&path("folder/one")).unwrap(), b"one");
}

#[test]
fn parent_watchdog_retires_the_worker_and_the_next_request_uses_a_fresh_one() {
    let fixture = Fixture::new();
    let executor = SubprocessBash::new(
        BashConfig::new(env!("CARGO_BIN_EXE_vsh-bash-worker"))
            .with_wall_timeout(Duration::from_millis(100)),
    )
    .unwrap();
    let started = Instant::now();
    assert!(matches!(
        executor.execute(
            "sleep 10",
            &mut fixture.filesystem(),
            &CallPolicy::default(),
            ExecutionLimits::default()
        ),
        Err(BashError::Timeout)
    ));
    assert!(started.elapsed() < Duration::from_secs(2));
    let result = executor
        .execute(
            "printf clean",
            &mut fixture.filesystem(),
            &CallPolicy::default(),
            ExecutionLimits::default(),
        )
        .unwrap();
    assert_eq!(result.stdout, b"clean");
}

#[test]
fn excluded_authority_and_stub_commands_fail_closed_even_when_nested_or_caught() {
    let fixture = Fixture::new();
    let executor = executor();
    for code in [
        "curl https://example.invalid || true; printf changed > output.txt",
        "wget https://example.invalid || true; true",
        "http https://example.invalid || true; true",
        "command curl https://example.invalid || true; true",
        "eval 'wget https://example.invalid || true'; true",
        "f() { http https://example.invalid || true; }; f; true",
        "sh -c 'curl https://example.invalid || true'; true",
        "chown 0 input.txt || true; true",
        "command chown 0 input.txt || true; true",
        "retry 3 printf fake || true; true",
        "watch printf fake || true; true",
        "kill 1 || true; true",
        "fc -s old=new || true; true",
        "env ssh example.invalid || true; true",
        "command env FOO=bar curl https://example.invalid || true; true",
        "tar -xf archive.tar || true; true",
        "cp -s input.txt output.txt || true; true",
        "cp --preserve=all input.txt output.txt || true; true",
        "mv -n input.txt output.txt || true; true",
        "command command cp -l input.txt output.txt || true; true",
        "eval 'cp --attributes-only input.txt output.txt || true'; true",
        "cp -r folder output || true; true",
        "chmod 700 --recursive folder || true; true",
        "chmod 600 -- -secret || true; true",
        "chmod u+s input.txt || true; true",
        "chmod +t input.txt || true; true",
        "rm -i input.txt || true; true",
        "command rm --interactive input.txt || true; true",
        "printf '\\377' || true; true",
        "echo -e '\\xff' || true; true",
        "printf '%c' 'é' > output.txt || true; true",
        "printf '%.1s' 'é' > output.txt || true; true",
        "printf '%.*s' 1 'é' > output.txt || true; true",
        "printf -v value '%.1s' 'é'; printf '%s' \"$value\" > output.txt || true; true",
        "command command printf -v value '%c' 'é' || true; true",
        "command command printf '%.1s' 'é' > output.txt || true; true",
    ] {
        assert!(
            executor
                .execute(
                    code,
                    &mut fixture.filesystem(),
                    &CallPolicy::default(),
                    ExecutionLimits::default()
                )
                .is_err(),
            "{code}"
        );
    }
    assert_eq!(
        executor
            .execute(
                "printf clean",
                &mut fixture.filesystem(),
                &CallPolicy::default(),
                ExecutionLimits::default()
            )
            .unwrap()
            .stdout,
        b"clean"
    );
}

#[test]
fn independent_worker_reports_its_exact_build_identity() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_vsh-bash-worker"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        vsh_bash::WORKER_VERSION
    );
}

#[test]
fn cancellation_is_sticky_and_retires_the_active_worker() {
    let fixture = Fixture::new();
    let executor = executor();
    let cancelled = BashCancellation::default();
    assert!(cancelled.cancel());
    let mut filesystem = fixture.filesystem();
    assert!(matches!(
        executor.execute_cancellable(
            "printf no > result",
            &mut filesystem,
            &CallPolicy::default(),
            ExecutionLimits::default(),
            &cancelled
        ),
        Err(BashError::Cancelled)
    ));
    assert!(filesystem.effects().is_empty());
    let running = BashCancellation::default();
    let started = Instant::now();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            std::thread::sleep(Duration::from_millis(100));
            assert!(running.cancel());
        });
        assert!(matches!(
            executor.execute_cancellable(
                "printf partial > result; sleep 10; printf late > result",
                &mut filesystem,
                &CallPolicy::default(),
                ExecutionLimits::default(),
                &running
            ),
            Err(BashError::Cancelled)
        ));
    });
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        executor
            .execute(
                "printf fresh",
                &mut fixture.filesystem(),
                &CallPolicy::default(),
                ExecutionLimits::default()
            )
            .unwrap()
            .stdout,
        b"fresh"
    );
}

#[test]
fn virtual_coprocesses_are_sequential_and_nested_process_substitution_is_terminal() {
    let fixture = Fixture::new();
    let executor = executor();
    for code in [
        "coproc { printf first > result; }; printf second >> result",
        "eval 'coproc { printf first > result; }'; printf second >> result",
        "f() { coproc { printf first > result; }; }; f; printf second >> result",
        "sh -c 'coproc { printf first > result; }'; printf second >> result",
    ] {
        let mut filesystem = fixture.filesystem();
        executor
            .execute(
                code,
                &mut filesystem,
                &CallPolicy::default(),
                ExecutionLimits::default(),
            )
            .unwrap();
        assert_eq!(
            filesystem.read(&path("result")).unwrap(),
            b"firstsecond",
            "{code}"
        );
    }
    for code in [
        "cat <(printf data) || true; true",
        "eval 'cat <(printf data) || true'; true",
        "sh -c 'cat <(printf data) || true'; true",
        "printf 'cat <(printf data) || true' > nested.sh; source nested.sh; true",
    ] {
        assert!(
            executor
                .execute(
                    code,
                    &mut fixture.filesystem(),
                    &CallPolicy::default(),
                    ExecutionLimits::default()
                )
                .is_err(),
            "{code}"
        );
    }
}

#[test]
fn nested_intermediate_and_allocator_ceilings_cannot_report_complete_success() {
    let fixture = Fixture::new();
    let limits = BashLimits {
        max_live_intermediate_bytes: 128,
        ..BashLimits::default()
    };
    let executor = SubprocessBash::new(
        BashConfig::new(env!("CARGO_BIN_EXE_vsh-bash-worker")).with_limits(limits),
    )
    .unwrap();
    assert!(
        executor
            .execute(
                "sh -c 'printf %02000d 1' | cat > output.txt; true",
                &mut fixture.filesystem(),
                &CallPolicy::default(),
                ExecutionLimits::default()
            )
            .is_err()
    );
    let executor =
        SubprocessBash::new(BashConfig::new(env!("CARGO_BIN_EXE_vsh-bash-worker"))).unwrap();
    assert!(
        executor
            .execute(
                "printf '%08000000d' 1 > /dev/null",
                &mut fixture.filesystem(),
                &CallPolicy::default(),
                ExecutionLimits {
                    max_memory_bytes: 64 * 1024,
                    ..ExecutionLimits::default()
                }
            )
            .is_err()
    );
    assert_eq!(
        executor
            .execute(
                "printf recovered",
                &mut fixture.filesystem(),
                &CallPolicy::default(),
                ExecutionLimits::default()
            )
            .unwrap()
            .stdout,
        b"recovered"
    );
}

#[test]
fn concurrent_sessions_have_disjoint_overlays_and_bounded_worker_checkout() {
    let fixture = Fixture::new();
    let executor = Arc::new(
        SubprocessBash::new(
            BashConfig::new(env!("CARGO_BIN_EXE_vsh-bash-worker")).with_worker_limits(2, 2),
        )
        .unwrap(),
    );
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|index| {
                let executor = Arc::clone(&executor);
                let base = fixture.base.clone();
                scope.spawn(move || {
                    let mut filesystem = VirtualFs::new(base);
                    executor
                        .execute(
                            &format!("printf {index} > output.txt"),
                            &mut filesystem,
                            &CallPolicy::default(),
                            ExecutionLimits::default(),
                        )
                        .unwrap();
                    assert_eq!(
                        filesystem.read(&path("output.txt")).unwrap(),
                        index.to_string().as_bytes()
                    );
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
}

#[test]
fn malformed_workers_and_diagnostic_floods_are_rejected_with_finite_teardown() {
    use std::os::unix::fs::PermissionsExt;
    let python = std::process::Command::new("python3")
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .unwrap();
    assert!(python.status.success());
    let python = String::from_utf8(python.stdout).unwrap();
    let fixture_source = include_str!("fixtures/fake_worker.py")
        .replacen("#!/usr/bin/python3", &format!("#!{}", python.trim()), 1)
        .replace("vsh/0.5.0", concat!("vsh/", env!("CARGO_PKG_VERSION")));
    for mode in [
        "bad_version",
        "oversized_frame",
        "truncated_frame",
        "bad_nested_length",
        "wrong_direction",
        "wrong_session",
        "double_finish",
        "call_after_finish",
        "stderr_flood",
        "blocked_write",
    ] {
        let fixture = Fixture::new();
        let fake_worker = fixture.directory.join(mode);
        fs::write(&fake_worker, &fixture_source).unwrap();
        fs::set_permissions(&fake_worker, fs::Permissions::from_mode(0o700)).unwrap();
        let launched = Instant::now();
        let config = BashConfig::new(fake_worker).with_wall_timeout(Duration::from_millis(100));
        match SubprocessBash::new(config) {
            Err(error) => {
                assert_eq!(
                    mode, "bad_version",
                    "unexpected constructor failure: {error}"
                );
                assert!(
                    launched.elapsed() < Duration::from_secs(6),
                    "handshake exceeds its five-second boundary"
                );
            }
            Ok(executor) => {
                let started = Instant::now();
                let code = if mode == "blocked_write" {
                    "#".repeat(512 * 1024)
                } else {
                    "true".into()
                };
                assert!(
                    executor
                        .execute(
                            &code,
                            &mut fixture.filesystem(),
                            &CallPolicy::default(),
                            ExecutionLimits::default()
                        )
                        .is_err(),
                    "malformed worker succeeded: {mode}"
                );
                drop(executor);
                assert!(
                    started.elapsed() < Duration::from_secs(2),
                    "unbounded teardown: {mode}"
                );
            }
        }
    }
}
