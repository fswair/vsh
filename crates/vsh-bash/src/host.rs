use std::error::Error;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use vsh_execution::{
    ExecutionBudget, ExecutionLimitExceeded, ExecutionLimits, ExecutionStats, FsGateway,
    GatewayError, VirtualRoot, check_path_bytes,
};
use vsh_policy::{CallPolicy, DeniedAccess};
use vsh_types::{NodeKind, NodeState, RuntimeConfigDigest};
use vsh_vfs::{EffectOrigin, VfsError, VirtualFs};

use crate::protocol::{
    self, BashLimits, Frame, FsFault, FsRequest, FsValue, HARD_FRAME_BYTES, MAX_DIAGNOSTIC_BYTES,
    Message, Metadata, PROFILE, Run, WORKER_ID,
};

static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);
const MAX_REUSE: u64 = 64;
const CONTROL_POLL: Duration = Duration::from_millis(20);

impl Default for BashConfig {
    fn default() -> Self {
        Self::new(
            std::env::var_os("VSH_BASH_WORKER")
                .filter(|path| !path.is_empty())
                .map_or_else(|| PathBuf::from("vsh-bash-worker"), PathBuf::from),
        )
    }
}

/// Request-scoped cooperative cancellation for the private execution backend.
/// Cancellation is sticky; use a fresh handle for each request. A blocking host
/// filesystem operation is checked on return, not forcibly interrupted.
pub type BashCancellation = vsh_execution::ExecutionCancellation;

struct ExecutionControl<'a> {
    deadline: Instant,
    cancellation: &'a BashCancellation,
}

impl ExecutionControl<'_> {
    fn remaining(&self) -> Result<Duration, BashError> {
        if self.cancellation.is_cancelled() {
            return Err(BashError::Cancelled);
        }
        self.deadline
            .checked_duration_since(Instant::now())
            .ok_or(BashError::Timeout)
    }

    fn receive<T>(&self, receiver: &mpsc::Receiver<T>) -> Result<Option<T>, BashError> {
        loop {
            match receiver.recv_timeout(self.remaining()?.min(CONTROL_POLL)) {
                Ok(value) => {
                    self.remaining()?;
                    return Ok(Some(value));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(None),
            }
        }
    }
}

/// Trusted host configuration for a separate, bounded Bashkit worker pool.
#[derive(Clone, Debug)]
pub struct BashConfig {
    worker_path: PathBuf,
    limits: BashLimits,
    wall_timeout: Option<Duration>,
    max_active_workers: usize,
    max_idle_workers: usize,
}

impl BashConfig {
    /// Use an explicitly supplied, matching VSH worker executable.
    #[must_use]
    pub fn new(worker_path: impl Into<PathBuf>) -> Self {
        Self {
            worker_path: worker_path.into(),
            limits: BashLimits::default(),
            wall_timeout: None,
            max_active_workers: 4,
            max_idle_workers: 4,
        }
    }

    /// Override interpreter work ceilings without changing filesystem authority.
    #[must_use]
    pub const fn with_limits(mut self, limits: BashLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Override the host watchdog, including worker checkout and execution.
    #[must_use]
    pub const fn with_wall_timeout(mut self, timeout: Duration) -> Self {
        self.wall_timeout = Some(timeout);
        self
    }

    /// Bound concurrent workers and retained reset workers. Zero idle disables reuse.
    #[must_use]
    pub const fn with_worker_limits(mut self, active: usize, idle: usize) -> Self {
        self.max_active_workers = active;
        self.max_idle_workers = idle;
        self
    }

    /// Return the trusted executable location; this is never given to the guest.
    #[must_use]
    pub fn worker_path(&self) -> &Path {
        &self.worker_path
    }

    /// Return the configured guest ceilings.
    #[must_use]
    pub const fn limits(&self) -> BashLimits {
        self.limits
    }

    fn timeout(&self, limits: ExecutionLimits) -> Duration {
        self.wall_timeout
            .unwrap_or(limits.max_duration.saturating_add(Duration::from_secs(1)))
    }

    /// Bind frontend, profile, transport, environment and all execution limits.
    #[must_use]
    pub fn security_digest(&self, limits: ExecutionLimits) -> RuntimeConfigDigest {
        let mut bytes = Vec::new();
        for value in [
            WORKER_ID,
            PROFILE,
            vsh_execution::FS_GATEWAY_VERSION,
            "/workspace",
            "PATH=/workspace;HOME=/workspace;LANG=C;LC_ALL=C;TZ=UTC",
        ] {
            bytes.extend_from_slice(&(value.len() as u64).to_le_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        for value in [
            self.limits.max_work_units,
            self.limits.max_aggregate_input_bytes,
            self.limits.max_live_intermediate_bytes,
            self.limits.max_commands as u64,
            self.limits.max_loop_iterations as u64,
            self.limits.max_total_loop_iterations as u64,
            self.limits.max_parser_operations as u64,
            limits.max_program_bytes as u64,
            limits.max_recursion_depth as u64,
            limits.max_memory_bytes as u64,
            limits.max_os_calls,
            limits.max_read_bytes,
            limits.max_write_bytes,
            limits.max_io_call_bytes as u64,
            limits.max_path_bytes as u64,
            limits.max_directory_entries,
            limits.max_evidence_records,
            limits.max_evidence_bytes,
            limits.max_output_bytes as u64,
            limits.max_result_bytes as u64,
            limits.max_exception_bytes as u64,
            self.max_active_workers as u64,
            self.max_idle_workers as u64,
            MAX_REUSE,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&limits.max_duration.as_nanos().to_le_bytes());
        bytes.extend_from_slice(&self.timeout(limits).as_nanos().to_le_bytes());
        RuntimeConfigDigest::digest_canonical(&bytes)
    }
}

/// Byte-authoritative guest result and parent-authoritative filesystem observations.
#[derive(Clone, Debug)]
pub struct BashOutcome {
    /// Final shell status. A nonzero status is never an executable commit proposal.
    pub exit_code: i32,
    /// Complete, bounded stdout bytes, not a lossy text projection.
    pub stdout: Vec<u8>,
    /// Complete, bounded stderr bytes.
    pub stderr: Vec<u8>,
    /// Shared filesystem budget counters.
    pub stats: ExecutionStats,
    /// Sticky policy denials even if Bash handled the individual error.
    pub denied_accesses: Vec<DeniedAccess>,
}

/// A terminal execution failure. None of these outcomes can authorize a commit.
#[derive(Debug)]
pub enum BashError {
    /// Invalid trusted configuration.
    Configuration(String),
    /// Worker transport, launch or termination failed.
    Io(io::Error),
    /// Worker violated the private stateful protocol.
    Protocol(&'static str),
    /// Parent watchdog or worker checkout deadline expired.
    Timeout,
    /// The request was cancelled; partial virtual state must be discarded.
    Cancelled,
    /// Parent-authoritative work or output ceiling was exceeded.
    Limit(ExecutionLimitExceeded),
    /// A guest attempted an unsupported command, namespace or filesystem feature.
    Unsupported(String),
    /// Parsing, interpreter execution, or a guest resource ceiling failed.
    Execution(String),
    /// Final status is nonzero; the virtual changes remain noncommittable.
    Exit {
        /// Final shell status.
        code: i32,
        /// Bounded complete stdout.
        stdout: Vec<u8>,
        /// Bounded complete stderr.
        stderr: Vec<u8>,
        /// Parent-authoritative denials retained even when the shell fails.
        denied_accesses: Vec<DeniedAccess>,
    },
}

impl fmt::Display for BashError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(detail) => {
                write!(formatter, "invalid Bash configuration: {detail}")
            }
            Self::Io(error) => write!(formatter, "Bash worker I/O: {error}"),
            Self::Protocol(detail) => write!(formatter, "Bash worker protocol: {detail}"),
            Self::Timeout => formatter.write_str("Bash parent deadline expired"),
            Self::Cancelled => formatter.write_str("Bash request cancelled"),
            Self::Limit(error) => write!(formatter, "Bash filesystem/output limit: {error}"),
            Self::Unsupported(detail) => {
                write!(formatter, "unsupported Bash profile operation: {detail}")
            }
            Self::Execution(detail) => write!(formatter, "Bash guest execution failed: {detail}"),
            Self::Exit { code, .. } => write!(
                formatter,
                "Bash exited with status {code}; virtual changes cannot be committed"
            ),
        }
    }
}
impl Error for BashError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Limit(error) => Some(error),
            _ => None,
        }
    }
}
impl From<io::Error> for BashError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<ExecutionLimitExceeded> for BashError {
    fn from(value: ExecutionLimitExceeded) -> Self {
        Self::Limit(value)
    }
}

#[derive(Default)]
struct Pool {
    idle: Vec<Worker>,
    active: usize,
}

/// Synchronous host boundary; Tokio and Bashkit live exclusively in child processes.
pub struct SubprocessBash {
    config: BashConfig,
    pool: Mutex<Pool>,
    available: Condvar,
}

impl SubprocessBash {
    /// Validate and prewarm one matching worker without passing host authority to it.
    ///
    /// # Errors
    /// Returns an invalid configuration, launch, version or handshake failure.
    pub fn new(config: BashConfig) -> Result<Self, BashError> {
        Self::new_cancellable(config, &BashCancellation::default())
    }

    /// Validate and prewarm with the request's cancellation control.
    ///
    /// # Errors
    /// Returns startup failures or cancellation without retaining a live worker.
    pub fn new_cancellable(
        mut config: BashConfig,
        cancellation: &BashCancellation,
    ) -> Result<Self, BashError> {
        if !cfg!(unix) {
            return Err(BashError::Configuration(
                "the initial Bash profile requires a Unix host".into(),
            ));
        }
        if config.max_active_workers == 0 || config.max_idle_workers > config.max_active_workers {
            return Err(BashError::Configuration(
                "active workers must be positive and idle workers cannot exceed active workers"
                    .into(),
            ));
        }
        if !config.worker_path.is_file() {
            return Err(BashError::Configuration(
                "an explicit matching worker executable is required".into(),
            ));
        }
        // Resolve against the caller's directory before the child changes cwd.
        // Retain this absolute selection for every subsequent pool spawn too.
        config.worker_path = config.worker_path.canonicalize()?;
        let worker = Worker::spawn(
            &config.worker_path,
            &ExecutionControl {
                deadline: Instant::now() + Duration::from_secs(5),
                cancellation,
            },
        )?;
        let mut pool = Pool::default();
        if config.max_idle_workers > 0 {
            pool.idle.push(worker);
        }
        Ok(Self {
            config,
            pool: Mutex::new(pool),
            available: Condvar::new(),
        })
    }

    /// Return the complete host configuration used in transaction identity.
    #[must_use]
    pub const fn config(&self) -> &BashConfig {
        &self.config
    }

    /// Execute against one borrowed parent VFS through its policy-aware gateway.
    ///
    /// The caller must discard failed virtual state and use VSH's normal policy,
    /// binding, approval, stale revalidation and commit pipeline after success.
    ///
    /// # Errors
    /// Returns a terminal failure for profile/resource/protocol errors or nonzero exit.
    pub fn execute(
        &self,
        code: &str,
        filesystem: &mut VirtualFs,
        policy: &CallPolicy,
        limits: ExecutionLimits,
    ) -> Result<BashOutcome, BashError> {
        self.execute_cancellable(
            code,
            filesystem,
            policy,
            limits,
            &BashCancellation::default(),
        )
    }

    /// Execute with a host-owned request cancellation handle.
    ///
    /// # Errors
    /// Same terminal failures as [`Self::execute`], plus [`BashError::Cancelled`].
    pub fn execute_cancellable(
        &self,
        code: &str,
        filesystem: &mut VirtualFs,
        policy: &CallPolicy,
        limits: ExecutionLimits,
        cancellation: &BashCancellation,
    ) -> Result<BashOutcome, BashError> {
        let deadline = Instant::now()
            .checked_add(self.config.timeout(limits))
            .ok_or_else(|| BashError::Configuration("wall deadline overflow".into()))?;
        let control = ExecutionControl {
            deadline,
            cancellation,
        };
        control.remaining()?;
        limits.check_program_bytes(code.len())?;
        if limits.max_memory_bytes == 0
            || limits.max_output_bytes > HARD_FRAME_BYTES / 2
            || limits.max_program_bytes > HARD_FRAME_BYTES / 2
        {
            return Err(BashError::Configuration(
                "invalid memory/program/output ceiling".into(),
            ));
        }
        filesystem.limit_evidence(limits.evidence_limits());
        filesystem
            .check_evidence()
            .map_err(|source| BashError::Execution(source.to_string()))?;
        let mut lease = self.checkout(&control)?;
        let worker = lease
            .worker
            .as_mut()
            .ok_or(BashError::Protocol("checked-out worker is missing"))?;
        let result = worker.execute(
            code,
            filesystem,
            policy,
            limits,
            self.config.limits,
            &control,
        );
        control.remaining()?;
        if result.is_ok() && worker.executions < MAX_REUSE {
            lease.reusable = true;
        }
        result
    }

    fn checkout(&self, control: &ExecutionControl<'_>) -> Result<Lease<'_>, BashError> {
        let mut pool = self
            .pool
            .lock()
            .map_err(|_| BashError::Protocol("worker pool lock poisoned"))?;
        while pool.active >= self.config.max_active_workers {
            let remaining = control.remaining()?.min(CONTROL_POLL);
            pool = self
                .available
                .wait_timeout(pool, remaining)
                .map_err(|_| BashError::Protocol("worker pool lock poisoned"))?
                .0;
        }
        control.remaining()?;
        pool.active += 1;
        let idle = pool.idle.pop();
        drop(pool);
        let mut lease = Lease {
            owner: self,
            worker: idle,
            reusable: false,
        };
        if lease.worker.is_none() {
            lease.worker = Some(Worker::spawn(&self.config.worker_path, control)?);
        }
        Ok(lease)
    }
}

struct Lease<'a> {
    owner: &'a SubprocessBash,
    worker: Option<Worker>,
    reusable: bool,
}
impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if let Ok(mut pool) = self.owner.pool.lock() {
            if self.reusable
                && pool.idle.len() < self.owner.config.max_idle_workers
                && let Some(worker) = self.worker.take()
            {
                pool.idle.push(worker);
            }
            pool.active = pool.active.saturating_sub(1);
            self.owner.available.notify_one();
        }
    }
}

struct WriteRequest {
    frame: Frame,
    maximum: usize,
    completion: mpsc::SyncSender<io::Result<()>>,
}

struct Worker {
    process: Child,
    writes: mpsc::SyncSender<WriteRequest>,
    events: mpsc::Receiver<io::Result<Frame>>,
    decode_limits: Arc<Mutex<protocol::DecodeLimits>>,
    diagnostic: Arc<Mutex<Vec<u8>>>,
    stderr_overflow: Arc<AtomicBool>,
    reader: Option<thread::JoinHandle<()>>,
    writer: Option<thread::JoinHandle<()>>,
    stderr_reader: Option<thread::JoinHandle<()>>,
    executions: u64,
    clean: bool,
}

impl Worker {
    fn command(path: &Path) -> Command {
        let mut command = Command::new(path);
        command
            .arg("--worker")
            .env_clear()
            .env("LANG", "C")
            .env("LC_ALL", "C")
            .env("TZ", "UTC")
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Only instrumented test builds inherit a profile destination. Guest
        // shell environment still comes exclusively from the fixed profile.
        #[cfg(coverage)]
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        command
    }

    fn spawn(path: &Path, control: &ExecutionControl<'_>) -> Result<Self, BashError> {
        control.remaining()?;
        let mut process = Self::command(path).spawn()?;
        let mut input = process
            .stdin
            .take()
            .ok_or(BashError::Protocol("worker has no stdin"))?;
        let mut output = process
            .stdout
            .take()
            .ok_or(BashError::Protocol("worker has no stdout"))?;
        let mut stderr = process
            .stderr
            .take()
            .ok_or(BashError::Protocol("worker has no stderr"))?;
        let decode_limits = Arc::new(Mutex::new(protocol::DecodeLimits {
            frame_bytes: MAX_DIAGNOSTIC_BYTES + 64,
            io_bytes: 0,
            path_bytes: 0,
            output_bytes: 0,
            messages: 1 << 1,
        }));
        let (writes, outgoing) = mpsc::sync_channel::<WriteRequest>(1);
        let write_thread = thread::spawn(move || {
            while let Ok(request) = outgoing.recv() {
                let result = protocol::write_frame(&mut input, &request.frame, request.maximum);
                let failed = result.is_err();
                let _ = request.completion.send(result);
                if failed {
                    break;
                }
            }
        });
        let bounds = Arc::clone(&decode_limits);
        let (sender, events) = mpsc::sync_channel(1);
        let diagnostics_sender = sender.clone();
        let reader = thread::spawn(move || {
            loop {
                let event = protocol::read_frame_with_limits(&mut output, || {
                    bounds
                        .lock()
                        .map_or(protocol::DecodeLimits::all(0), |value| *value)
                });
                let failed = event.is_err();
                if sender.send(event).is_err() || failed {
                    break;
                }
            }
        });
        let diagnostic = Arc::new(Mutex::new(Vec::new()));
        let stderr_overflow = Arc::new(AtomicBool::new(false));
        let overflow = Arc::clone(&stderr_overflow);
        let observed = Arc::clone(&diagnostic);
        let stderr_reader = thread::spawn(move || {
            let mut buffer = [0; 1024];
            while let Ok(length) = stderr.read(&mut buffer) {
                if length == 0 {
                    break;
                }
                if let Ok(mut retained) = observed.lock() {
                    let remaining = MAX_DIAGNOSTIC_BYTES.saturating_sub(retained.len());
                    retained.extend_from_slice(&buffer[..length.min(remaining)]);
                    if length > remaining && !overflow.swap(true, Ordering::AcqRel) {
                        let _ = diagnostics_sender.try_send(Err(protocol::invalid(
                            "worker diagnostic stream exceeded its ceiling",
                        )));
                    }
                }
            }
        });
        let mut worker = Self {
            process,
            writes,
            events,
            decode_limits,
            diagnostic,
            stderr_overflow,
            reader: Some(reader),
            writer: Some(write_thread),
            stderr_reader: Some(stderr_reader),
            executions: 0,
            clean: false,
        };
        let hello = worker.receive(control)?;
        if hello.session != 0
            || hello.sequence != 0
            || !matches!(hello.message, Message::Hello(ref id) if id == WORKER_ID)
        {
            return Err(BashError::Protocol("worker identity/handshake mismatch"));
        }
        worker.clean = true;
        Ok(worker)
    }

    fn receive(&mut self, control: &ExecutionControl<'_>) -> Result<Frame, BashError> {
        if self.stderr_overflow.load(Ordering::Acquire) {
            return Err(BashError::Protocol(
                "worker diagnostic stream exceeded its ceiling",
            ));
        }
        if let Some(event) = control.receive(&self.events)? {
            if self.stderr_overflow.load(Ordering::Acquire) {
                return Err(BashError::Protocol(
                    "worker diagnostic stream exceeded its ceiling",
                ));
            }
            event.map_err(BashError::Io)
        } else {
            let detail = self
                .diagnostic
                .lock()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
                .unwrap_or_default();
            Err(BashError::Execution(format!("worker terminated: {detail}")))
        }
    }

    fn send(
        &self,
        frame: Frame,
        maximum: usize,
        control: &ExecutionControl<'_>,
    ) -> Result<(), BashError> {
        control.remaining()?;
        let (completion, written) = mpsc::sync_channel(1);
        self.writes
            .try_send(WriteRequest {
                frame,
                maximum,
                completion,
            })
            .map_err(|_| BashError::Protocol("worker write queue closed or unexpectedly full"))?;
        match control.receive(&written)? {
            Some(result) => result.map_err(BashError::Io),
            None => Err(BashError::Protocol("worker write thread terminated")),
        }
    }

    #[expect(
        clippy::too_many_lines,
        reason = "Keep the stateful worker session and its terminal branches together"
    )]
    fn execute(
        &mut self,
        code: &str,
        filesystem: &mut VirtualFs,
        policy: &CallPolicy,
        limits: ExecutionLimits,
        bash_limits: BashLimits,
        control: &ExecutionControl<'_>,
    ) -> Result<BashOutcome, BashError> {
        let session = NEXT_SESSION
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| BashError::Protocol("session identifier exhausted"))?;
        let cap = limits
            .max_io_call_bytes
            .max(limits.max_output_bytes)
            .max(limits.max_result_bytes)
            .saturating_add(limits.max_path_bytes.saturating_mul(2))
            .saturating_add(MAX_DIAGNOSTIC_BYTES + 256)
            .min(HARD_FRAME_BYTES);
        *self
            .decode_limits
            .lock()
            .map_err(|_| BashError::Protocol("decoder bounds lock poisoned"))? =
            protocol::DecodeLimits {
                frame_bytes: cap,
                io_bytes: limits.max_io_call_bytes,
                path_bytes: limits.max_path_bytes,
                output_bytes: limits.max_output_bytes,
                messages: (1 << 3) | (1 << 5) | (1 << 6),
            };
        self.clean = false;
        self.send(
            Frame {
                session,
                sequence: 0,
                message: Message::Run(Run {
                    code: code.to_owned(),
                    limits: bash_limits,
                    duration_us: u64::try_from(limits.max_duration.as_micros()).unwrap_or(u64::MAX),
                    max_program_bytes: limits.max_program_bytes,
                    max_memory_bytes: limits.max_memory_bytes,
                    max_output_bytes: limits.max_output_bytes,
                    max_recursion_depth: limits.max_recursion_depth,
                }),
            },
            HARD_FRAME_BYTES,
            control,
        )?;
        let mut sequence = 1;
        let mut budget = ExecutionBudget::new(limits);
        let mut denied_accesses = Vec::new();
        let root = VirtualRoot::default();
        let mut profile_failure = None;
        loop {
            let frame = self.receive(control)?;
            if frame.session != session || frame.sequence != sequence {
                return Err(BashError::Protocol("out-of-order request or wrong session"));
            }
            match frame.message {
                Message::Call(request) => {
                    control.remaining()?;
                    budget.charge_os_call()?;
                    let response = dispatch(request, filesystem, policy, &mut budget, &root);
                    control.remaining()?;
                    let reply = match response {
                        Ok(value) => Ok(value),
                        Err(DispatchError::Gateway(GatewayError::Policy(denial))) => {
                            budget.record_denial();
                            let detail = format!("filesystem access denied: {denial:?}");
                            vsh_execution::retain_denial(
                                filesystem,
                                limits,
                                &mut denied_accesses,
                                *denial,
                            )
                            .map_err(|source| match source {
                                GatewayError::Limit(source) => BashError::Limit(source),
                                source => BashError::Execution(source.to_string()),
                            })?;
                            Err(FsFault {
                                kind: 4,
                                detail: bounded(detail),
                            })
                        }
                        Err(DispatchError::Gateway(GatewayError::Limit(limit))) => {
                            return Err(limit.into());
                        }
                        Err(DispatchError::Gateway(GatewayError::Filesystem(error))) => {
                            match *error {
                                error @ (VfsError::Snapshot(_)
                                | VfsError::Store(_)
                                | VfsError::EvidenceLimit(_)
                                | VfsError::Path(_)) => {
                                    return Err(BashError::Execution(bounded(error.to_string())));
                                }
                                error @ (VfsError::SymlinkMode { .. }
                                | VfsError::UnsupportedMode { .. }
                                | VfsError::NotFile {
                                    actual: NodeKind::Symlink,
                                    ..
                                }
                                | VfsError::NotDirectory {
                                    actual: NodeKind::Symlink,
                                    ..
                                }) => {
                                    let detail = bounded(error.to_string());
                                    profile_failure.get_or_insert_with(|| detail.clone());
                                    Err(FsFault { kind: 8, detail })
                                }
                                error => Err(filesystem_fault(&error)),
                            }
                        }
                        Err(DispatchError::Gateway(error)) => {
                            return Err(BashError::Execution(bounded(error.to_string())));
                        }
                        Err(DispatchError::Unsupported(detail)) => {
                            profile_failure.get_or_insert_with(|| detail.clone());
                            Err(FsFault {
                                kind: 8,
                                detail: bounded(detail),
                            })
                        }
                        Err(DispatchError::Limit(limit)) => return Err(limit.into()),
                    };
                    self.send(
                        Frame {
                            session,
                            sequence,
                            message: Message::Reply(reply),
                        },
                        cap,
                        control,
                    )?;
                    sequence = sequence
                        .checked_add(1)
                        .ok_or(BashError::Protocol("request identifier exhausted"))?;
                }
                Message::Done {
                    exit_code,
                    stdout,
                    stderr,
                    failure,
                } => {
                    let output_bytes = stdout
                        .len()
                        .checked_add(stderr.len())
                        .ok_or(BashError::Protocol("output size overflow"))?;
                    if output_bytes > limits.max_output_bytes {
                        return Err(ExecutionLimitExceeded::OutputBytes {
                            limit: limits.max_output_bytes as u64,
                            attempted: output_bytes as u64,
                        }
                        .into());
                    }
                    let ready = self.receive(control)?;
                    if ready.session != session
                        || ready.sequence != sequence
                        || !matches!(ready.message, Message::Ready)
                    {
                        return Err(BashError::Protocol(
                            "worker did not confirm fresh interpreter reset",
                        ));
                    }
                    self.executions += 1;
                    self.clean = true;
                    if let Some(detail) = profile_failure {
                        return Err(BashError::Unsupported(detail));
                    }
                    if let Some(failure) = failure {
                        return Err(BashError::Execution(failure));
                    }
                    if exit_code != 0 {
                        return Err(BashError::Exit {
                            code: exit_code,
                            stdout,
                            stderr,
                            denied_accesses,
                        });
                    }
                    let mut stats = budget.stats();
                    stats.output_bytes = output_bytes;
                    stats.result_bytes = 4;
                    if limits.max_result_bytes < 4 {
                        return Err(ExecutionLimitExceeded::ResultBytes {
                            limit: limits.max_result_bytes as u64,
                            attempted: 4,
                        }
                        .into());
                    }
                    return Ok(BashOutcome {
                        exit_code,
                        stdout,
                        stderr,
                        stats,
                        denied_accesses,
                    });
                }
                _ => return Err(BashError::Protocol("unexpected event in running session")),
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if self.clean
            && self
                .send(
                    Frame {
                        session: 0,
                        sequence: 0,
                        message: Message::Shutdown,
                    },
                    64,
                    &ExecutionControl {
                        deadline: Instant::now() + Duration::from_millis(50),
                        cancellation: &BashCancellation::default(),
                    },
                )
                .is_ok()
        {
            // A clean worker exits normally; the finite grace period also lets
            // coverage instrumentation flush without weakening failure teardown.
            for _ in 0..10 {
                if self.process.try_wait().ok().flatten().is_some() {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
        let _ = self.process.kill();
        let _ = self.process.wait();
        // Dropping the receiver before joining releases a malicious sender blocked
        // on backpressure. Child termination closes both operating-system pipes.
        let (_, closed) = mpsc::channel();
        let old = std::mem::replace(&mut self.events, closed);
        drop(old);
        let (closed, _) = mpsc::sync_channel(1);
        let old = std::mem::replace(&mut self.writes, closed);
        drop(old);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
        if let Some(writer) = self.writer.take() {
            let _ = writer.join();
        }
    }
}

enum DispatchError {
    Gateway(GatewayError),
    Unsupported(String),
    Limit(ExecutionLimitExceeded),
}
impl From<GatewayError> for DispatchError {
    fn from(value: GatewayError) -> Self {
        Self::Gateway(value)
    }
}
impl From<ExecutionLimitExceeded> for DispatchError {
    fn from(value: ExecutionLimitExceeded) -> Self {
        Self::Limit(value)
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "One exhaustive RPC operation match is the filesystem authority boundary"
)]
fn dispatch(
    request: FsRequest,
    filesystem: &mut VirtualFs,
    policy: &CallPolicy,
    budget: &mut ExecutionBudget,
    root: &VirtualRoot,
) -> Result<FsValue, DispatchError> {
    let map = |path: &str| {
        check_path_bytes(path, budget.limits())?;
        // POSIX backslashes are literal names. The initial portable namespace
        // rejects them rather than applying Monty's Windows-path normalization.
        if path.contains('\\') {
            return Err(DispatchError::Unsupported(
                "backslash paths are outside the initial portable Bash profile".into(),
            ));
        }
        root.map_path(path).map_err(|_| {
            DispatchError::Unsupported(bounded(format!(
                "path is outside the workspace namespace: {path}"
            )))
        })
    };
    if let FsRequest::Unsupported(path, operation) = request {
        return Err(DispatchError::Unsupported(bounded(format!(
            "{operation} is unsupported at {path}"
        ))));
    }
    if let FsRequest::Stat(ref path) = request
        && path == "/"
    {
        return Ok(FsValue::Metadata(Metadata {
            kind: 2,
            size: 0,
            mode: 0o755,
        }));
    }
    if let FsRequest::Exists(ref path) = request
        && path == "/"
    {
        return Ok(FsValue::Bool(true));
    }
    let first = match &request {
        FsRequest::Read(p)
        | FsRequest::Write(p, _)
        | FsRequest::Append(p, _)
        | FsRequest::Mkdir(p, _)
        | FsRequest::Remove(p, _)
        | FsRequest::Stat(p)
        | FsRequest::ReadDir(p)
        | FsRequest::Exists(p)
        | FsRequest::Rename(p, _)
        | FsRequest::Copy(p, _)
        | FsRequest::ReadLink(p)
        | FsRequest::Chmod(p, _) => map(p)?,
        FsRequest::Unsupported(_, _) => unreachable!("unsupported operations returned above"),
    };
    let second = match &request {
        FsRequest::Rename(_, p) | FsRequest::Copy(_, p) => Some(map(p)?),
        _ => None,
    };
    let limits = budget.limits();
    let mut gateway = FsGateway::new(filesystem, policy, budget, EffectOrigin::BashCall);
    Ok(match request {
        FsRequest::Read(_) => FsValue::Bytes(gateway.read(&first)?),
        FsRequest::Write(_, bytes) => {
            gateway.write(&first, &bytes)?;
            FsValue::Unit
        }
        FsRequest::Append(_, bytes) => {
            gateway.append(&first, &bytes)?;
            FsValue::Unit
        }
        FsRequest::Mkdir(_, recursive) => {
            gateway.mkdir_options(&first, 0o755, recursive, recursive)?;
            FsValue::Unit
        }
        FsRequest::Remove(_, recursive) => {
            gateway.remove(&first, recursive, false)?;
            FsValue::Unit
        }
        FsRequest::Stat(_) => FsValue::Metadata(wire_metadata(gateway.metadata(&first)?)),
        FsRequest::Exists(_) => match gateway.metadata(&first) {
            Ok(_) => FsValue::Bool(true),
            Err(GatewayError::Filesystem(error)) if matches!(*error, VfsError::NotFound { .. }) => {
                FsValue::Bool(false)
            }
            Err(error) => return Err(error.into()),
        },
        FsRequest::ReadDir(_) => {
            let children = gateway.read_dir(&first)?;
            let length = children
                .iter()
                .try_fold(8_usize, |used, path| {
                    used.checked_add(21 + path.file_name().map_or(0, str::len))
                })
                .unwrap_or(usize::MAX);
            if length > limits.max_result_bytes {
                return Err(ExecutionLimitExceeded::ResultBytes {
                    limit: limits.max_result_bytes as u64,
                    attempted: length as u64,
                }
                .into());
            }
            let mut entries = Vec::with_capacity(children.len());
            for child in children {
                let state = gateway.metadata(&child)?;
                entries.push((
                    child
                        .file_name()
                        .expect("directory child has a name")
                        .to_owned(),
                    wire_metadata(state),
                ));
            }
            FsValue::Entries(entries)
        }
        FsRequest::Rename(_, _) => {
            gateway.rename(&first, second.as_ref().expect("mapped destination"))?;
            FsValue::Unit
        }
        FsRequest::Copy(_, _) => {
            gateway.copy(
                &first,
                second.as_ref().expect("mapped destination"),
                false,
                true,
            )?;
            FsValue::Unit
        }
        FsRequest::ReadLink(_) => FsValue::Bytes(gateway.read_link(&first)?),
        FsRequest::Chmod(_, mode) => {
            if mode & !0o777 != 0 {
                return Err(DispatchError::Unsupported(format!(
                    "unsupported chmod bits: {mode:o}"
                )));
            }
            gateway.set_mode(&first, mode)?;
            FsValue::Unit
        }
        FsRequest::Unsupported(_, _) => unreachable!("unsupported operations returned above"),
    })
}

fn wire_metadata(state: NodeState) -> Metadata {
    Metadata {
        kind: match state.kind() {
            NodeKind::File => 1,
            NodeKind::Directory => 2,
            NodeKind::Symlink => 3,
        },
        size: state.size(),
        mode: state.mode(),
    }
}

fn filesystem_fault(error: &VfsError) -> FsFault {
    let kind = match error {
        VfsError::NotFound { .. } => 1,
        VfsError::AlreadyExists { .. } => 2,
        VfsError::NotDirectory { .. } => 3,
        VfsError::DirectoryNotEmpty { .. } => 5,
        VfsError::IsDirectory { .. } => 6,
        _ => 7,
    };
    FsFault {
        kind,
        detail: bounded(error.to_string()),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_checkout_does_not_reserve_or_spawn_a_worker() {
        let executor = SubprocessBash {
            config: BashConfig::new("unused").with_worker_limits(1, 0),
            pool: Mutex::new(Pool {
                active: 1,
                idle: Vec::new(),
            }),
            available: Condvar::new(),
        };
        let cancellation = BashCancellation::default();
        let control = ExecutionControl {
            deadline: Instant::now() + Duration::from_secs(10),
            cancellation: &cancellation,
        };
        let started = Instant::now();
        thread::scope(|scope| {
            scope.spawn(|| {
                thread::sleep(Duration::from_millis(50));
                assert!(cancellation.cancel());
            });
            assert!(matches!(
                executor.checkout(&control),
                Err(BashError::Cancelled)
            ));
        });
        assert!(started.elapsed() < Duration::from_secs(2));
        assert_eq!(executor.pool.lock().unwrap().active, 1);
    }

    #[test]
    fn cancelled_channel_wait_cannot_accept_a_late_success() {
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.send(42).unwrap();
        let cancellation = BashCancellation::default();
        assert!(cancellation.cancel());
        let control = ExecutionControl {
            deadline: Instant::now() + Duration::from_secs(10),
            cancellation: &cancellation,
        };
        assert!(matches!(
            control.receive(&receiver),
            Err(BashError::Cancelled)
        ));
    }
}
