//! Child-only adapter. There is no host filesystem implementation in this module.

use std::collections::BTreeSet;
use std::io::{self, Stdin, Stdout};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use bashkit::{
    Bash, CommandResolver, DirEntry, FileSystem, FileSystemExt, FileType, Metadata, Result,
    async_trait, hooks::HookAction,
};

use crate::protocol::{
    self, Frame, FsRequest, FsValue, HARD_FRAME_BYTES, MAX_DIAGNOSTIC_BYTES, Message, Run,
    WORKER_ID,
};

// These builtins either require excluded authority or acknowledge operations
// they do not actually perform. A catchable shell status is not sufficient.
fn excluded_command(name: &str) -> bool {
    matches!(
        name,
        "parallel"
            | "df"
            | "curl"
            | "wget"
            | "http"
            | "chown"
            | "kill"
            | "retry"
            | "watch"
            | "fc"
            | "env"
            | "tar"
    )
}

fn unsupported_invocation<'a>(
    mut name: &'a str,
    mut args: &'a [String],
    supported: &OnceLock<BTreeSet<String>>,
) -> Option<String> {
    // Inspect actual dispatched argv, including builtin-to-builtin `command`
    // forwarding. This does not parse shell source or reimplement a builtin.
    if name == "command" {
        let flags = args
            .iter()
            .take_while(|arg| matches!(arg.as_str(), "-p" | "-v" | "-V"))
            .count();
        if args[..flags]
            .iter()
            .any(|arg| matches!(arg.as_str(), "-v" | "-V"))
        {
            return None;
        }
        let (target, remaining) = args[flags..].split_first()?;
        if !supported.get().is_some_and(|names| names.contains(target)) {
            return Some(bounded(format!("unsupported command target: {target}")));
        }
        name = target;
        args = remaining;
    }
    if excluded_command(name) {
        return Some(format!(
            "{name} is not implemented by the initial VSH profile"
        ));
    }
    if matches!(name, "cp" | "mv" | "rm") {
        for arg in args.iter().take_while(|arg| arg.as_str() != "--") {
            if arg == "-" || !arg.starts_with('-') || matches!(arg.as_str(), "--help" | "--version")
            {
                continue;
            }
            if name == "rm"
                && (matches!(arg.as_str(), "--recursive" | "--force")
                    || arg.strip_prefix('-').is_some_and(|flags| {
                        flags.chars().all(|flag| matches!(flag, 'r' | 'R' | 'f'))
                    }))
            {
                continue;
            }
            return Some(bounded(format!("unsupported {name} option: {arg}")));
        }
    }
    if name == "chmod" && args.iter().skip(1).any(|arg| arg.starts_with('-')) {
        return Some("chmod accepts explicit paths only; prefix dash-leading names with ./".into());
    }
    if name == "printf" {
        if args.first().is_some_and(|argument| argument == "-v") {
            return Some("printf variable assignment (-v) is unsupported".into());
        }
        // This is an argv compatibility contract, not a shell/format interpreter.
        // Upstream precision and %c slice UTF-8 by byte, then replace invalid bytes.
        let mut format = args.first().map_or("", String::as_str).chars();
        while let Some(character) = format.next() {
            if character == '%' && !matches!(format.next(), Some('%' | 's')) {
                return Some("printf supports literal text, %s and %% only; use byte-preserving file streams".into());
            }
        }
    }
    // Upstream text builtins can replace or remove numeric byte escapes.
    // Reject the dispatched argv instead of sealing silently changed bytes.
    if matches!(name, "printf" | "echo")
        && args.iter().any(|arg| {
            arg.as_bytes()
                .windows(2)
                .any(|pair| pair[0] == b'\\' && matches!(pair[1], b'x' | b'0'..=b'7'))
        })
    {
        return Some(format!(
            "{name} numeric byte escapes are unsupported; use byte-preserving file streams"
        ));
    }
    if name == "chmod" && args.first().is_some_and(|mode| mode.contains(['s', 't'])) {
        return Some("chmod special permission bits are unsupported".into());
    }
    None
}

struct Transport {
    input: Stdin,
    output: Stdout,
    session: u64,
    sequence: u64,
    failure: Option<String>,
}
struct RpcFs {
    transport: Arc<Mutex<Transport>>,
}

impl RpcFs {
    fn request(&self, request: FsRequest) -> Result<FsValue> {
        let mut transport = self
            .transport
            .lock()
            .map_err(|_| io::Error::other("RPC lock poisoned"))?;
        let session = transport.session;
        let sequence = transport.sequence;
        let frame = Frame {
            session,
            sequence,
            message: Message::Call(request),
        };
        protocol::write_frame(&mut transport.output, &frame, HARD_FRAME_BYTES)?;
        let reply = protocol::read_frame(&mut transport.input, HARD_FRAME_BYTES)?;
        if reply.session != session || reply.sequence != sequence {
            transport
                .failure
                .get_or_insert_with(|| "filesystem reply identity mismatch".into());
            return Err(io::Error::other("filesystem reply identity mismatch").into());
        }
        transport.sequence = sequence
            .checked_add(1)
            .ok_or_else(|| io::Error::other("RPC sequence overflow"))?;
        let Message::Reply(value) = reply.message else {
            transport
                .failure
                .get_or_insert_with(|| "unexpected filesystem reply kind".into());
            return Err(io::Error::other("unexpected filesystem reply kind").into());
        };
        value.map_err(|fault| {
            let kind = match fault.kind {
                1 => io::ErrorKind::NotFound,
                2 => io::ErrorKind::AlreadyExists,
                3 => io::ErrorKind::NotADirectory,
                4 | 8 => io::ErrorKind::PermissionDenied,
                5 => io::ErrorKind::DirectoryNotEmpty,
                6 => io::ErrorKind::IsADirectory,
                _ => io::ErrorKind::Other,
            };
            io::Error::new(kind, fault.detail).into()
        })
    }

    fn unit(&self, request: FsRequest) -> Result<()> {
        match self.request(request)? {
            FsValue::Unit => Ok(()),
            _ => Err(io::Error::other("unexpected filesystem result").into()),
        }
    }

    fn path(path: &Path) -> Result<String> {
        path.to_str().map(str::to_owned).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "non-UTF-8 virtual path").into()
        })
    }
}

#[async_trait]
impl FileSystemExt for RpcFs {
    fn usage(&self) -> bashkit::FsUsage {
        if let Ok(mut transport) = self.transport.lock() {
            transport
                .failure
                .get_or_insert_with(|| "filesystem-wide usage is unsupported".into());
        }
        bashkit::FsUsage::default()
    }

    async fn mkfifo(&self, path: &Path, _mode: u32) -> Result<()> {
        self.unit(FsRequest::Unsupported(
            Self::path(path)?,
            "FIFO creation".into(),
        ))
    }

    fn backend_kind(&self) -> &'static str {
        "vsh-parent-rpc"
    }
}

#[async_trait]
impl FileSystem for RpcFs {
    async fn read_file(&self, path: &Path) -> Result<Vec<u8>> {
        match self.request(FsRequest::Read(Self::path(path)?))? {
            FsValue::Bytes(bytes) => Ok(bytes),
            _ => Err(io::Error::other("unexpected read result").into()),
        }
    }
    async fn write_file(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.unit(FsRequest::Write(Self::path(path)?, bytes.to_vec()))
    }
    async fn append_file(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        self.unit(FsRequest::Append(Self::path(path)?, bytes.to_vec()))
    }
    async fn mkdir(&self, path: &Path, recursive: bool) -> Result<()> {
        self.unit(FsRequest::Mkdir(Self::path(path)?, recursive))
    }
    async fn remove(&self, path: &Path, recursive: bool) -> Result<()> {
        self.unit(FsRequest::Remove(Self::path(path)?, recursive))
    }
    async fn stat(&self, path: &Path) -> Result<Metadata> {
        match self.request(FsRequest::Stat(Self::path(path)?))? {
            FsValue::Metadata(state) => metadata(state),
            _ => Err(io::Error::other("unexpected stat result").into()),
        }
    }
    async fn read_dir(&self, path: &Path) -> Result<Vec<DirEntry>> {
        match self.request(FsRequest::ReadDir(Self::path(path)?))? {
            FsValue::Entries(entries) => entries
                .into_iter()
                .map(|(name, state)| {
                    Ok(DirEntry {
                        name,
                        metadata: metadata(state)?,
                    })
                })
                .collect(),
            _ => Err(io::Error::other("unexpected directory result").into()),
        }
    }
    async fn exists(&self, path: &Path) -> Result<bool> {
        match self.request(FsRequest::Exists(Self::path(path)?))? {
            FsValue::Bool(value) => Ok(value),
            _ => Err(io::Error::other("unexpected exists result").into()),
        }
    }
    async fn rename(&self, from: &Path, to: &Path) -> Result<()> {
        self.unit(FsRequest::Rename(Self::path(from)?, Self::path(to)?))
    }
    async fn copy(&self, from: &Path, to: &Path) -> Result<()> {
        self.unit(FsRequest::Copy(Self::path(from)?, Self::path(to)?))
    }
    async fn symlink(&self, _target: &Path, link: &Path) -> Result<()> {
        self.unit(FsRequest::Unsupported(
            Self::path(link)?,
            "symlink creation".into(),
        ))
    }
    async fn read_link(&self, path: &Path) -> Result<PathBuf> {
        match self.request(FsRequest::ReadLink(Self::path(path)?))? {
            FsValue::Bytes(bytes) => String::from_utf8(bytes).map(PathBuf::from).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "non-UTF-8 opaque link target").into()
            }),
            _ => Err(io::Error::other("unexpected readlink result").into()),
        }
    }
    async fn chmod(&self, path: &Path, mode: u32) -> Result<()> {
        self.unit(FsRequest::Chmod(Self::path(path)?, mode))
    }
    async fn set_modified_time(&self, path: &Path, _time: SystemTime) -> Result<()> {
        self.unit(FsRequest::Unsupported(
            Self::path(path)?,
            "timestamp mutation".into(),
        ))
    }
}

fn metadata(state: protocol::Metadata) -> Result<Metadata> {
    let file_type = match state.kind {
        1 => FileType::File,
        2 => FileType::Directory,
        3 => FileType::Symlink,
        _ => return Err(io::Error::other("invalid metadata kind").into()),
    };
    Ok(Metadata {
        file_type,
        size: state.size,
        mode: state.mode,
        modified: SystemTime::UNIX_EPOCH,
        created: SystemTime::UNIX_EPOCH,
    })
}

struct DenyUnknown {
    failure: Arc<Mutex<Option<String>>>,
}
impl CommandResolver for DenyUnknown {
    fn resolve(&self, name: &str) -> Option<Arc<dyn bashkit::Builtin>> {
        if let Ok(mut failure) = self.failure.lock() {
            failure.get_or_insert_with(|| bounded(format!("unsupported command: {name}")));
        }
        None
    }
}

fn execute(
    run: &Run,
    transport: &Arc<Mutex<Transport>>,
    runtime: &tokio::runtime::Runtime,
) -> (i32, Vec<u8>, Vec<u8>, Option<String>) {
    let filesystem = Arc::new(RpcFs {
        transport: Arc::clone(transport),
    });
    let failure = Arc::new(Mutex::new(None));
    let intercepted = Arc::clone(&failure);
    let builtin_names = Arc::new(OnceLock::<BTreeSet<String>>::new());
    let supported = Arc::clone(&builtin_names);
    let mut limits = bashkit::ExecutionLimits::default();
    limits.max_work_units = run.limits.max_work_units;
    limits.max_aggregate_input_bytes = run.limits.max_aggregate_input_bytes;
    limits.max_live_intermediate_bytes = run.limits.max_live_intermediate_bytes;
    limits.max_commands = run.limits.max_commands;
    limits.max_loop_iterations = run.limits.max_loop_iterations;
    limits.max_total_loop_iterations = run.limits.max_total_loop_iterations;
    limits.max_parser_operations = run.limits.max_parser_operations;
    limits.timeout = Duration::from_micros(run.duration_us);
    limits.parser_timeout = limits.timeout;
    limits.max_input_bytes = run.max_program_bytes;
    limits.max_ast_depth = 64;
    limits.max_function_depth = run.max_recursion_depth.min(64);
    limits.max_subst_depth = run.max_recursion_depth.min(16);
    limits.max_subshell_depth = run.max_recursion_depth.min(16);
    // These upstream caps truncate nested producer data without a fatal flag.
    // The sticky shared work/intermediate budget and hard allocator bound buffers;
    // parent retention limits and its watchdog remain authoritative.
    limits.max_stdout_bytes = usize::MAX;
    limits.max_stderr_bytes = usize::MAX;
    limits.max_file_descriptors = 128;
    limits.max_history_entries = 0;
    limits.max_history_bytes = 0;
    limits.max_history_output_bytes = 0;
    limits.max_word_split_fields = 100_000;
    limits.max_word_split_bytes = 4 * 1024 * 1024;
    limits.capture_final_env = false;
    let mut bash = Bash::builder()
        .fs(filesystem.clone())
        .cwd("/workspace")
        .env("PATH", "/workspace")
        .env("HOME", "/workspace")
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .env("VSH_PROFILE", protocol::PROFILE)
        .limits(limits)
        .before_tool(Box::new(move |event| {
            if let Some(detail) = unsupported_invocation(&event.name, &event.args, &supported) {
                if let Ok(mut failure) = intercepted.lock() {
                    failure.get_or_insert_with(|| detail.clone());
                }
                HookAction::Cancel(detail)
            } else {
                HookAction::Continue(event)
            }
        }))
        .command_resolver(Arc::new(DenyUnknown {
            failure: Arc::clone(&failure),
        }))
        .build();
    let _ = builtin_names.set(bash.builtin_names().into_iter().collect());
    let result = runtime.block_on(bash.exec(&run.code));
    let profile_failure = failure.lock().ok().and_then(|value| value.clone());
    let outcome = match result {
        Ok(result) => {
            let stdout = result.stdout.as_bytes().to_vec();
            let stderr = result.stderr.as_bytes().to_vec();
            let oversized = stdout
                .len()
                .checked_add(stderr.len())
                .is_none_or(|length| length > run.max_output_bytes);
            if oversized {
                (
                    1,
                    Vec::new(),
                    Vec::new(),
                    Some("parent-retained output ceiling exceeded".into()),
                )
            } else {
                (result.exit_code, stdout, stderr, profile_failure)
            }
        }
        Err(error) => (
            1,
            Vec::new(),
            Vec::new(),
            Some(profile_failure.unwrap_or_else(|| bounded(error.to_string()))),
        ),
    };
    drop(bash);
    drop(filesystem);
    outcome
}

/// Run the private child protocol. Build only into the isolated worker executable.
///
/// # Errors
/// Returns a bounded protocol/I/O error; guest failures are ordinary result frames.
pub fn worker_main() -> io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()?;
    monty_alloc::set_limit(None, false).map_err(io::Error::other)?;
    let transport = Arc::new(Mutex::new(Transport {
        input: io::stdin(),
        output: io::stdout(),
        session: 0,
        sequence: 0,
        failure: None,
    }));
    {
        let mut io = transport
            .lock()
            .map_err(|_| io::Error::other("RPC lock poisoned"))?;
        protocol::write_frame(
            &mut io.output,
            &Frame {
                session: 0,
                sequence: 0,
                message: Message::Hello(WORKER_ID.into()),
            },
            MAX_DIAGNOSTIC_BYTES + 64,
        )?;
    }
    let mut last_session = 0;
    loop {
        let frame = {
            let mut io = transport
                .lock()
                .map_err(|_| io::Error::other("RPC lock poisoned"))?;
            protocol::read_frame(&mut io.input, HARD_FRAME_BYTES)?
        };
        if matches!(frame.message, Message::Shutdown) && frame.session == 0 && frame.sequence == 0 {
            return Ok(());
        }
        if frame.session <= last_session || frame.sequence != 0 {
            return Err(protocol::invalid("invalid session start"));
        }
        let Message::Run(run) = frame.message else {
            return Err(protocol::invalid("expected fresh execution request"));
        };
        last_session = frame.session;
        monty_alloc::set_limit(Some(run.max_memory_bytes), false).map_err(io::Error::other)?;
        {
            let mut io = transport
                .lock()
                .map_err(|_| io::Error::other("RPC lock poisoned"))?;
            io.session = frame.session;
            io.sequence = 1;
            io.failure = None;
        }
        let (exit_code, stdout, stderr, mut failure) = execute(&run, &transport, &runtime);
        monty_alloc::set_limit(None, false).map_err(io::Error::other)?;
        if Arc::strong_count(&transport) != 1 {
            return Err(protocol::invalid(
                "guest retained filesystem transport after reset",
            ));
        }
        let mut io = transport
            .lock()
            .map_err(|_| io::Error::other("RPC lock poisoned"))?;
        if let Some(error) = io.failure.take() {
            failure.get_or_insert(error);
        }
        let sequence = io.sequence;
        protocol::write_frame(
            &mut io.output,
            &Frame {
                session: frame.session,
                sequence,
                message: Message::Done {
                    exit_code,
                    stdout,
                    stderr,
                    failure,
                },
            },
            HARD_FRAME_BYTES,
        )?;
        protocol::write_frame(
            &mut io.output,
            &Frame {
                session: frame.session,
                sequence,
                message: Message::Ready,
            },
            64,
        )?;
    }
}

fn bounded(mut detail: String) -> String {
    if detail.len() > MAX_DIAGNOSTIC_BYTES {
        let mut length = MAX_DIAGNOSTIC_BYTES;
        while !detail.is_char_boundary(length) {
            length -= 1;
        }
        detail.truncate(length);
    }
    detail
}
