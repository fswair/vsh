use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use monty_proto::{MAX_FRAME_LEN, PROTOCOL_VERSION, decode_frame, pb, write_frame};
use monty_types::{ExtFunctionResult, MontyException};
use vsh_types::RuntimeConfigDigest;
use vsh_vfs::{EffectOrigin, VirtualFs};

use super::{
    Budget, CallFailure, DeniedAccess, ExecutionError, ExecutionLimitExceeded, ExecutionOutcome,
    InProcessConfig, MontyFailurePhase, WorkerFailure, WorkerFailureKind, dispatch_call,
    encode_string, exception_bytes, limit_error, measure_result, permission_denied,
};

const EXPECTED_MONTY_VERSION: &str = "0.0.21";
const MAX_WORKER_DIAGNOSTIC_BYTES: usize = 4 * 1024;
const VERSION_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const FRAME_OVERHEAD_BYTES: usize = 64 * 1024;
const MAX_WORKER_EVENTS_PER_EXECUTION: u64 = 16_384;

/// Configuration for the crash-isolated, typed Monty subprocess adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubprocessConfig {
    pub(super) adapter: InProcessConfig,
    worker_path: PathBuf,
    wall_timeout_override: Option<Duration>,
    max_idle_workers: usize,
}

impl SubprocessConfig {
    /// Bind the adapter to an exact `vsh-monty-worker` 0.0.21 executable.
    #[must_use]
    pub fn new(worker_path: impl Into<PathBuf>, adapter: InProcessConfig) -> Self {
        Self {
            adapter,
            worker_path: worker_path.into(),
            wall_timeout_override: None,
            max_idle_workers: 4,
        }
    }

    /// Replace the parent wall-clock watchdog for one complete execution.
    #[must_use]
    pub const fn with_wall_timeout(mut self, wall_timeout: Duration) -> Self {
        self.wall_timeout_override = Some(wall_timeout);
        self
    }

    /// Bound clean reset workers retained for warm reuse. Zero disables pooling.
    #[must_use]
    pub const fn with_max_idle_workers(mut self, max_idle_workers: usize) -> Self {
        self.max_idle_workers = max_idle_workers;
        self
    }

    /// Return the exact configured worker executable path.
    #[must_use]
    pub fn worker_path(&self) -> &Path {
        &self.worker_path
    }

    /// Return the shared typed-call and sandbox configuration.
    #[must_use]
    pub const fn adapter(&self) -> &InProcessConfig {
        &self.adapter
    }

    /// Hash every behavior-affecting worker setting for transaction binding.
    #[must_use]
    pub fn security_digest(&self) -> RuntimeConfigDigest {
        self.security_digest_for(&self.adapter)
    }

    /// Hash the worker boundary together with one request's typed adapter settings.
    #[must_use]
    pub fn security_digest_for(&self, adapter: &InProcessConfig) -> RuntimeConfigDigest {
        let mut canonical = Vec::new();
        encode_string("vsh-monty-subprocess-v1", &mut canonical);
        canonical.extend_from_slice(adapter.security_digest().as_bytes());
        encode_string("monty-runtime-0.0.21", &mut canonical);
        canonical.extend_from_slice(&self.wall_timeout(adapter).as_nanos().to_le_bytes());
        RuntimeConfigDigest::digest_canonical(&canonical)
    }

    fn wall_timeout(&self, adapter: &InProcessConfig) -> Duration {
        self.wall_timeout_override.unwrap_or_else(|| {
            adapter
                .limits
                .max_duration
                .saturating_add(Duration::from_secs(1))
        })
    }
}

/// A short-lock worker pool backed by Monty's official typed subprocess protocol.
///
/// The pool mutex is held only while popping or returning an idle process. Monty
/// bytecode, typed VFS calls, result conversion, and reset never execute under it.
#[derive(Debug)]
pub struct SubprocessMonty {
    config: SubprocessConfig,
    idle: Mutex<Vec<Worker>>,
}

impl SubprocessMonty {
    /// Validate the configured binary version before accepting any hostile code.
    ///
    /// # Errors
    ///
    /// Returns a worker spawn error unless the executable reports exact Monty 0.0.21.
    pub fn new(config: SubprocessConfig) -> Result<Self, ExecutionError> {
        if config
            .wall_timeout_override
            .is_some_and(|timeout| timeout.is_zero())
        {
            return Err(worker_error(
                WorkerFailureKind::Spawn,
                "worker wall timeout must be greater than zero",
            ));
        }
        verify_worker_version(&config.worker_path)?;
        Ok(Self {
            config,
            idle: Mutex::new(Vec::new()),
        })
    }

    /// Return the complete worker configuration.
    #[must_use]
    pub const fn config(&self) -> &SubprocessConfig {
        &self.config
    }

    /// Number of clean reset workers currently retained for warm reuse.
    ///
    /// # Errors
    ///
    /// Returns a worker failure if a previous panic poisoned the short pool lock.
    pub fn idle_workers(&self) -> Result<usize, ExecutionError> {
        Ok(self.pool()?.len())
    }

    /// Execute source in a supervised child against a caller-owned virtual transaction.
    ///
    /// # Errors
    ///
    /// Returns typed Monty, host budget, VFS integrity, protocol, crash, or watchdog
    /// failures. A worker is reused only after a successful clean reset; resource and
    /// infrastructure failures always discard it.
    pub fn execute(
        &self,
        code: impl Into<String>,
        filesystem: &mut VirtualFs,
    ) -> Result<ExecutionOutcome, ExecutionError> {
        self.execute_with_config(code, filesystem, &self.config.adapter)
    }

    /// Execute with request-specific limits and policy while reusing this worker pool.
    ///
    /// # Errors
    ///
    /// Returns the same typed failures as [`Self::execute`].
    pub fn execute_with_config(
        &self,
        code: impl Into<String>,
        filesystem: &mut VirtualFs,
        adapter: &InProcessConfig,
    ) -> Result<ExecutionOutcome, ExecutionError> {
        let code = code.into();
        let program_bytes = u64::try_from(code.len()).unwrap_or(u64::MAX);
        let max_program_bytes = u64::try_from(adapter.limits.max_program_bytes).unwrap_or(u64::MAX);
        if program_bytes > max_program_bytes {
            return Err(limit_error(ExecutionLimitExceeded::ProgramBytes {
                limit: max_program_bytes,
                attempted: program_bytes,
            }));
        }

        let mut worker = self.checkout()?;
        let wall_timeout = self.config.wall_timeout(adapter);
        let result = worker.execute(&code, filesystem, adapter, wall_timeout);
        let session_is_reusable = matches!(
            result,
            Ok(_)
                | Err(ExecutionError::Monty { .. } | ExecutionError::UnsupportedSuspension { .. })
        );
        if session_is_reusable && worker.reset(wall_timeout).is_ok() {
            self.checkin(worker)?;
        }
        result
    }

    fn pool(&self) -> Result<MutexGuard<'_, Vec<Worker>>, ExecutionError> {
        self.idle
            .lock()
            .map_err(|_| worker_error(WorkerFailureKind::Crashed, "worker pool lock was poisoned"))
    }

    fn checkout(&self) -> Result<Worker, ExecutionError> {
        if let Some(worker) = self.pool()?.pop() {
            Ok(worker)
        } else {
            Worker::spawn(&self.config.worker_path)
        }
    }

    fn checkin(&self, worker: Worker) -> Result<(), ExecutionError> {
        let mut pool = self.pool()?;
        if pool.len() < self.config.max_idle_workers {
            pool.push(worker);
        }
        Ok(())
    }
}

#[derive(Debug)]
enum WorkerMessage {
    Event(pb::ChildEvent),
    Eof,
    ReadError(String),
    FrameLimit {
        kind: Option<u32>,
        length: u32,
        limit: u32,
    },
}

#[derive(Debug)]
enum WorkerReadError {
    Detail(String),
    FrameLimit {
        kind: Option<u32>,
        length: u32,
        limit: u32,
    },
}

impl From<String> for WorkerReadError {
    fn from(detail: String) -> Self {
        Self::Detail(detail)
    }
}

#[derive(Debug)]
struct WorkerFrameLimits {
    hard: AtomicU32,
    output: AtomicU32,
    call: AtomicU32,
    result: AtomicU32,
    exception: AtomicU32,
    control: AtomicU32,
    output_bytes: AtomicU64,
    result_bytes: AtomicU64,
    exception_bytes: AtomicU64,
}

impl WorkerFrameLimits {
    fn new() -> Self {
        let initial = frame_cap(FRAME_OVERHEAD_BYTES);
        Self {
            hard: AtomicU32::new(initial),
            output: AtomicU32::new(initial),
            call: AtomicU32::new(initial),
            result: AtomicU32::new(initial),
            exception: AtomicU32::new(initial),
            control: AtomicU32::new(initial),
            output_bytes: AtomicU64::new(0),
            result_bytes: AtomicU64::new(0),
            exception_bytes: AtomicU64::new(0),
        }
    }

    fn configure(&self, adapter: &InProcessConfig) {
        let limits = adapter.limits;
        let output = frame_cap(limits.max_output_bytes);
        let call = frame_cap(
            limits
                .max_io_call_bytes
                .saturating_add(limits.max_path_bytes.saturating_mul(2)),
        );
        let result = frame_cap(limits.max_result_bytes);
        let exception = frame_cap(limits.max_exception_bytes);
        let control = frame_cap(limits.max_program_bytes.max(limits.max_path_bytes));
        self.output.store(output, Ordering::Relaxed);
        self.call.store(call, Ordering::Relaxed);
        self.result.store(result, Ordering::Relaxed);
        self.exception.store(exception, Ordering::Relaxed);
        self.control.store(control, Ordering::Relaxed);
        self.output_bytes.store(
            u64::try_from(limits.max_output_bytes).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        self.result_bytes.store(
            u64::try_from(limits.max_result_bytes).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        self.exception_bytes.store(
            u64::try_from(limits.max_exception_bytes).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        self.hard.store(
            output.max(call).max(result).max(exception).max(control),
            Ordering::Release,
        );
    }

    fn hard(&self) -> u32 {
        self.hard.load(Ordering::Acquire)
    }

    fn for_kind(&self, kind: Option<u32>) -> u32 {
        let limit = match kind {
            Some(1) => &self.output,
            Some(3) => &self.call,
            Some(6) => &self.result,
            Some(7) => &self.exception,
            _ => &self.control,
        };
        limit.load(Ordering::Relaxed)
    }

    fn semantic_limit(&self, kind: Option<u32>) -> Option<u64> {
        let limit = match kind {
            Some(1) => &self.output_bytes,
            Some(6) => &self.result_bytes,
            Some(7) => &self.exception_bytes,
            _ => return None,
        };
        Some(limit.load(Ordering::Relaxed))
    }
}

#[derive(Debug)]
struct Worker {
    process: Child,
    stdin: ChildStdin,
    events: Option<Receiver<WorkerMessage>>,
    reader: Option<JoinHandle<()>>,
    frame_limits: Arc<WorkerFrameLimits>,
}

impl Worker {
    fn spawn(path: &Path) -> Result<Self, ExecutionError> {
        let mut process = Command::new(path)
            .arg("subprocess")
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|source| {
                worker_error(
                    WorkerFailureKind::Spawn,
                    format!("cannot start {}: {source}", path.display()),
                )
            })?;
        let Some(stdin) = process.stdin.take() else {
            terminate_spawn(&mut process);
            return Err(worker_error(
                WorkerFailureKind::Spawn,
                "spawned worker has no stdin pipe",
            ));
        };
        let Some(stdout) = process.stdout.take() else {
            terminate_spawn(&mut process);
            return Err(worker_error(
                WorkerFailureKind::Spawn,
                "spawned worker has no stdout pipe",
            ));
        };
        let (sender, events) = mpsc::sync_channel(64);
        let frame_limits = Arc::new(WorkerFrameLimits::new());
        let reader_limits = Arc::clone(&frame_limits);
        let reader = thread::Builder::new()
            .name("vsh-monty-worker-reader".to_owned())
            .spawn(move || {
                let mut stdout = stdout;
                loop {
                    let message = match read_worker_event(&mut stdout, &reader_limits) {
                        Ok(Some(event)) => WorkerMessage::Event(event),
                        Ok(None) => WorkerMessage::Eof,
                        Err(WorkerReadError::Detail(source)) => WorkerMessage::ReadError(source),
                        Err(WorkerReadError::FrameLimit {
                            kind,
                            length,
                            limit,
                        }) => WorkerMessage::FrameLimit {
                            kind,
                            length,
                            limit,
                        },
                    };
                    let terminal = !matches!(message, WorkerMessage::Event(_));
                    if sender.send(message).is_err() || terminal {
                        break;
                    }
                }
            });
        let reader = match reader {
            Ok(reader) => reader,
            Err(source) => {
                terminate_spawn(&mut process);
                return Err(worker_error(
                    WorkerFailureKind::Spawn,
                    format!("cannot start worker reader: {source}"),
                ));
            }
        };
        Ok(Self {
            process,
            stdin,
            events: Some(events),
            reader: Some(reader),
            frame_limits,
        })
    }

    fn execute(
        &mut self,
        code: &str,
        filesystem: &mut VirtualFs,
        adapter: &InProcessConfig,
        wall_timeout: Duration,
    ) -> Result<ExecutionOutcome, ExecutionError> {
        self.frame_limits.configure(adapter);
        let deadline = Instant::now()
            .checked_add(wall_timeout)
            .ok_or_else(|| worker_error(WorkerFailureKind::Timeout, "worker deadline overflow"))?;
        self.configure(adapter, deadline)?;
        self.send(pb::parent_request::Kind::Feed(pb::Feed {
            code: code.to_owned(),
            inputs: Vec::new(),
            skip_type_check: true,
        }))?;

        let mut stdout = String::new();
        let mut budget = Budget::new(adapter.limits);
        let mut denied_accesses = Vec::new();
        let mut worker_events = 0_u64;
        loop {
            let event = self.receive(deadline)?;
            worker_events = worker_events.saturating_add(1);
            if worker_events > MAX_WORKER_EVENTS_PER_EXECUTION {
                return Err(worker_error(
                    WorkerFailureKind::Protocol,
                    "worker event limit exceeded",
                ));
            }
            let kind = event.kind.ok_or_else(|| {
                worker_error(WorkerFailureKind::Protocol, "child event has no kind")
            })?;
            match kind {
                pb::child_event::Kind::Print(output) => {
                    append_output(&mut stdout, &output, adapter.limits.max_output_bytes)?;
                }
                pb::child_event::Kind::OsCall(call) => {
                    self.resume_os_call(
                        call,
                        filesystem,
                        adapter,
                        &mut budget,
                        &mut denied_accesses,
                    )?;
                }
                pb::child_event::Kind::NameLookup(_) => {
                    self.send(pb::parent_request::Kind::ResumeNameLookup(
                        pb::ResumeNameLookup {
                            kind: Some(pb::resume_name_lookup::Kind::Undefined(pb::Unit {})),
                        },
                    ))?;
                }
                pb::child_event::Kind::Complete(complete) => {
                    let value = complete
                        .value
                        .ok_or_else(|| {
                            worker_error(WorkerFailureKind::Protocol, "complete event has no value")
                        })?
                        .into_object()
                        .map_err(|source| {
                            worker_error(WorkerFailureKind::Protocol, source.to_string())
                        })?;
                    let mut stats = budget.stats;
                    stats.output_bytes = stdout.len();
                    stats.result_bytes = measure_result(&value, adapter.limits.max_result_bytes)
                        .map_err(limit_error)?;
                    return Ok(ExecutionOutcome {
                        value,
                        stdout,
                        stats,
                        denied_accesses,
                    });
                }
                pb::child_event::Kind::Error(error) => {
                    return Err(worker_monty_error(
                        error,
                        adapter.limits.max_exception_bytes,
                    ));
                }
                pb::child_event::Kind::FunctionCall(call) => {
                    return Err(ExecutionError::UnsupportedSuspension {
                        kind: "external function call",
                        name: Some(call.function_name),
                    });
                }
                pb::child_event::Kind::ResolveFutures(_) => {
                    return Err(ExecutionError::UnsupportedSuspension {
                        kind: "future resolution",
                        name: None,
                    });
                }
                pb::child_event::Kind::FatalError(error) => {
                    return Err(worker_error(WorkerFailureKind::Crashed, error.message));
                }
                pb::child_event::Kind::TypingError(_)
                | pb::child_event::Kind::DumpResult(_)
                | pb::child_event::Kind::Ok(_)
                | pb::child_event::Kind::Shutdown(_) => {
                    return Err(worker_error(
                        WorkerFailureKind::Protocol,
                        "unexpected child event during execution",
                    ));
                }
            }
        }
    }

    fn configure(
        &mut self,
        adapter: &InProcessConfig,
        deadline: Instant,
    ) -> Result<(), ExecutionError> {
        self.send(pb::parent_request::Kind::Configure(pb::Configure {
            script_name: adapter.script_name.clone(),
            limits: Some(pb::ResourceLimits {
                max_duration_micros: Some(duration_micros(adapter.limits.max_duration)),
                max_memory_bytes: Some(
                    u64::try_from(adapter.limits.max_memory_bytes).unwrap_or(u64::MAX),
                ),
                gc_interval: None,
                max_recursion_depth: Some(
                    u64::try_from(adapter.limits.max_recursion_depth).unwrap_or(u64::MAX),
                ),
            }),
            type_check: false,
            type_check_stubs: None,
            monty_version: EXPECTED_MONTY_VERSION.to_owned(),
            assert_message_annotations: None,
            type_check_format: pb::TypeCheckFormat::Unspecified as i32,
            type_check_color: false,
            protocol_version: PROTOCOL_VERSION,
        }))?;
        self.expect_ok(deadline, "configure")
    }

    fn resume_os_call(
        &mut self,
        call: pb::OsCall,
        filesystem: &mut VirtualFs,
        config: &InProcessConfig,
        budget: &mut Budget,
        denied_accesses: &mut Vec<DeniedAccess>,
    ) -> Result<(), ExecutionError> {
        let typed_call = call
            .call
            .ok_or_else(|| worker_error(WorkerFailureKind::Protocol, "OS call has no typed arm"))?
            .try_into()
            .map_err(|source: monty_proto::ProtoConvertError| {
                worker_error(WorkerFailureKind::Protocol, source.to_string())
            })?;
        budget.charge_os_call().map_err(limit_error)?;
        let result = filesystem.with_effect_origin(EffectOrigin::MontyOsCall, |filesystem| {
            dispatch_call(&typed_call, filesystem, config, budget)
        });
        let result = match result {
            Ok(value) => ExtFunctionResult::Return(value),
            Err(CallFailure::Python(exception)) => ExtFunctionResult::Error(exception),
            Err(CallFailure::Policy(denial)) => {
                budget.stats.denied_accesses = budget.stats.denied_accesses.saturating_add(1);
                let exception = permission_denied(denial.path.as_str());
                denied_accesses.push(denial);
                ExtFunctionResult::Error(exception)
            }
            Err(CallFailure::Limit(source)) => return Err(limit_error(source)),
            Err(CallFailure::InternalVfs(source)) => {
                return Err(ExecutionError::InternalVfs(Box::new(source)));
            }
        };
        self.send(pb::parent_request::Kind::ResumeCall(pb::ResumeCall {
            call_id: call.call_id,
            result: Some(result.into()),
        }))
    }

    fn reset(&mut self, timeout: Duration) -> Result<(), ExecutionError> {
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| worker_error(WorkerFailureKind::Timeout, "reset deadline overflow"))?;
        self.send(pb::parent_request::Kind::Reset(pb::Reset {}))?;
        self.expect_ok(deadline, "reset")
    }

    fn expect_ok(
        &mut self,
        deadline: Instant,
        operation: &'static str,
    ) -> Result<(), ExecutionError> {
        let event = self.receive(deadline)?;
        match event.kind {
            Some(pb::child_event::Kind::Ok(_)) => Ok(()),
            Some(pb::child_event::Kind::FatalError(error)) => {
                Err(worker_error(WorkerFailureKind::Crashed, error.message))
            }
            _ => Err(worker_error(
                WorkerFailureKind::Protocol,
                format!("unexpected child response to {operation}"),
            )),
        }
    }

    fn send(&mut self, kind: pb::parent_request::Kind) -> Result<(), ExecutionError> {
        write_frame(
            &mut self.stdin,
            &pb::ParentRequest {
                trace_parent: None,
                kind: Some(kind),
            },
        )
        .map_err(|source| worker_error(WorkerFailureKind::Transport, source.to_string()))
    }

    fn receive(&mut self, deadline: Instant) -> Result<pb::ChildEvent, ExecutionError> {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let _ = self.process.kill();
            return Err(worker_error(
                WorkerFailureKind::Timeout,
                "worker wall-clock deadline exceeded",
            ));
        }
        let events = self.events.as_ref().ok_or_else(|| {
            worker_error(WorkerFailureKind::Crashed, "worker reader is unavailable")
        })?;
        match events.recv_timeout(remaining) {
            Ok(WorkerMessage::Event(event)) => Ok(event),
            Ok(WorkerMessage::Eof) => Err(worker_error(
                WorkerFailureKind::Crashed,
                "worker exited before a turn-ending event",
            )),
            Ok(WorkerMessage::ReadError(detail)) => {
                Err(worker_error(WorkerFailureKind::Transport, detail))
            }
            Ok(WorkerMessage::FrameLimit {
                kind,
                length,
                limit,
            }) => {
                let attempted = u64::from(length);
                match (kind, self.frame_limits.semantic_limit(kind)) {
                    (Some(1), Some(limit)) => {
                        Err(limit_error(ExecutionLimitExceeded::OutputBytes {
                            limit,
                            attempted,
                        }))
                    }
                    (Some(6), Some(limit)) => {
                        Err(limit_error(ExecutionLimitExceeded::ResultBytes {
                            limit,
                            attempted,
                        }))
                    }
                    (Some(7), Some(limit)) => {
                        Err(limit_error(ExecutionLimitExceeded::ExceptionBytes {
                            limit,
                            attempted,
                        }))
                    }
                    _ => Err(worker_error(
                        WorkerFailureKind::Transport,
                        format!(
                            "worker event kind {kind:?} has {length} wire bytes; request maximum is {limit}"
                        ),
                    )),
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                let _ = self.process.kill();
                Err(worker_error(
                    WorkerFailureKind::Timeout,
                    "worker wall-clock deadline exceeded",
                ))
            }
            Err(RecvTimeoutError::Disconnected) => Err(worker_error(
                WorkerFailureKind::Crashed,
                "worker reader disconnected",
            )),
        }
    }
}

fn frame_cap(payload_bytes: usize) -> u32 {
    u32::try_from(payload_bytes.saturating_add(FRAME_OVERHEAD_BYTES))
        .unwrap_or(u32::MAX)
        .min(MAX_FRAME_LEN)
}

fn read_worker_event(
    reader: &mut impl Read,
    limits: &WorkerFrameLimits,
) -> Result<Option<pb::ChildEvent>, WorkerReadError> {
    let mut length = [0_u8; 4];
    loop {
        match reader.read(&mut length[..1]) {
            Ok(0) => return Ok(None),
            Ok(1) => break,
            Ok(_) => unreachable!("one-byte read returned more than one byte"),
            Err(source) if source.kind() == std::io::ErrorKind::Interrupted => {}
            Err(source) => return Err(format!("frame prefix read failed: {source}").into()),
        }
    }
    reader
        .read_exact(&mut length[1..])
        .map_err(|source| format!("worker exited during frame prefix: {source}"))?;
    let length = u32::from_le_bytes(length);
    let hard_limit = limits.hard();
    if length > hard_limit {
        return Err(WorkerReadError::Detail(format!(
            "worker frame of {length} bytes exceeds request maximum of {hard_limit} bytes"
        )));
    }
    let length =
        usize::try_from(length).map_err(|_| "frame length does not fit host".to_owned())?;
    let mut body = Vec::new();
    body.try_reserve_exact(length)
        .map_err(|_| "cannot reserve bounded worker frame".to_owned())?;
    body.resize(length, 0);
    reader
        .read_exact(&mut body)
        .map_err(|source| format!("worker exited during frame body: {source}"))?;

    let kind = child_event_kind_tag(&body)?;
    let kind_limit = limits.for_kind(kind);
    if u32::try_from(length).unwrap_or(u32::MAX) > kind_limit {
        return Err(WorkerReadError::FrameLimit {
            kind,
            length: u32::try_from(length).unwrap_or(u32::MAX),
            limit: kind_limit,
        });
    }
    decode_frame(&body)
        .map(Some)
        .map_err(|source| WorkerReadError::Detail(format!("worker frame decode failed: {source}")))
}

fn child_event_kind_tag(bytes: &[u8]) -> Result<Option<u32>, String> {
    let mut offset = 0_usize;
    let mut kind = None;
    while offset < bytes.len() {
        let key = read_varint(bytes, &mut offset)?;
        if key == 0 {
            return Err("worker protobuf contains field key zero".to_owned());
        }
        let field = u32::try_from(key >> 3)
            .map_err(|_| "worker protobuf field number overflows u32".to_owned())?;
        let wire_type = u8::try_from(key & 0b111).expect("three bits fit u8");
        if (1..=12).contains(&field) {
            if wire_type != 2 {
                return Err(format!(
                    "worker event kind field {field} has invalid wire type {wire_type}"
                ));
            }
            if kind.replace(field).is_some() {
                return Err("worker protobuf contains multiple event kind fields".to_owned());
            }
        }
        skip_protobuf_value(bytes, &mut offset, wire_type)?;
    }
    Ok(kind)
}

fn read_varint(bytes: &[u8], offset: &mut usize) -> Result<u64, String> {
    let mut value = 0_u64;
    for shift in (0..=63).step_by(7) {
        let byte = *bytes
            .get(*offset)
            .ok_or_else(|| "truncated worker protobuf varint".to_owned())?;
        *offset = (*offset).saturating_add(1);
        if shift == 63 && byte > 1 {
            return Err("worker protobuf varint overflows u64".to_owned());
        }
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("worker protobuf varint is too long".to_owned())
}

fn skip_protobuf_value(bytes: &[u8], offset: &mut usize, wire_type: u8) -> Result<(), String> {
    let length = match wire_type {
        0 => {
            let _ = read_varint(bytes, offset)?;
            return Ok(());
        }
        1 => 8,
        2 => usize::try_from(read_varint(bytes, offset)?)
            .map_err(|_| "worker protobuf length does not fit host".to_owned())?,
        5 => 4,
        _ => return Err(format!("unsupported worker protobuf wire type {wire_type}")),
    };
    let end = offset
        .checked_add(length)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "truncated worker protobuf field".to_owned())?;
    *offset = end;
    Ok(())
}

fn terminate_spawn(process: &mut Child) {
    let _ = process.kill();
    let _ = process.wait();
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.events.take();
        let _ = self.process.kill();
        let _ = self.process.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn verify_worker_version(path: &Path) -> Result<(), ExecutionError> {
    let mut process = Command::new(path)
        .arg("--version")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| {
            worker_error(
                WorkerFailureKind::Spawn,
                format!("cannot inspect {}: {source}", path.display()),
            )
        })?;
    let deadline = Instant::now()
        .checked_add(VERSION_CHECK_TIMEOUT)
        .ok_or_else(|| worker_error(WorkerFailureKind::Spawn, "version deadline overflow"))?;
    let status = loop {
        match process.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                terminate_spawn(&mut process);
                return Err(worker_error(
                    WorkerFailureKind::Spawn,
                    format!(
                        "worker version check exceeded {} seconds",
                        VERSION_CHECK_TIMEOUT.as_secs()
                    ),
                ));
            }
            Err(source) => {
                terminate_spawn(&mut process);
                return Err(worker_error(
                    WorkerFailureKind::Spawn,
                    format!("cannot wait for worker version: {source}"),
                ));
            }
        }
    };
    let stdout = process.stdout.take().ok_or_else(|| {
        worker_error(
            WorkerFailureKind::Spawn,
            "version worker has no stdout pipe",
        )
    })?;
    let mut bytes = Vec::new();
    stdout
        .take(u64::try_from(MAX_WORKER_DIAGNOSTIC_BYTES + 1).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .map_err(|source| {
            worker_error(
                WorkerFailureKind::Spawn,
                format!("cannot read worker version: {source}"),
            )
        })?;
    if bytes.len() > MAX_WORKER_DIAGNOSTIC_BYTES {
        return Err(worker_error(
            WorkerFailureKind::Spawn,
            "worker version output exceeds 4096 bytes",
        ));
    }
    let stdout = String::from_utf8(bytes).map_err(|_| {
        worker_error(
            WorkerFailureKind::Spawn,
            "worker version output is not UTF-8",
        )
    })?;
    let version = stdout.split_whitespace().last();
    if !status.success() || version != Some(EXPECTED_MONTY_VERSION) {
        return Err(worker_error(
            WorkerFailureKind::Spawn,
            format!(
                "worker must report exact Monty {EXPECTED_MONTY_VERSION}; got {:?}",
                stdout.trim()
            ),
        ));
    }
    Ok(())
}

fn append_output(
    stdout: &mut String,
    output: &pb::Print,
    maximum: usize,
) -> Result<(), ExecutionError> {
    if pb::PrintStream::try_from(output.stream).ok() != Some(pb::PrintStream::Stdout) {
        return Err(worker_error(
            WorkerFailureKind::Protocol,
            "child emitted an unsupported print stream",
        ));
    }
    let attempted = stdout.len().saturating_add(output.text.len());
    if attempted > maximum {
        return Err(limit_error(ExecutionLimitExceeded::OutputBytes {
            limit: u64::try_from(maximum).unwrap_or(u64::MAX),
            attempted: u64::try_from(attempted).unwrap_or(u64::MAX),
        }));
    }
    stdout.push_str(&output.text);
    Ok(())
}

fn worker_monty_error(error: pb::Error, maximum: usize) -> ExecutionError {
    let exception = match error.exception {
        Some(exception) => MontyException::try_from(exception).map_err(|source| source.to_string()),
        None => Err("error event has no exception".to_owned()),
    };
    let source = match exception {
        Ok(source) => source,
        Err(detail) => return worker_error(WorkerFailureKind::Protocol, detail),
    };
    let attempted = exception_bytes(&source);
    let limit = u64::try_from(maximum).unwrap_or(u64::MAX);
    if attempted > limit {
        limit_error(ExecutionLimitExceeded::ExceptionBytes { limit, attempted })
    } else {
        ExecutionError::Monty {
            phase: MontyFailurePhase::Runtime,
            source: Box::new(source),
        }
    }
}

fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn worker_error(kind: WorkerFailureKind, detail: impl Into<String>) -> ExecutionError {
    ExecutionError::Worker(Box::new(WorkerFailure {
        kind,
        detail: bounded_detail(detail.into()),
    }))
}

fn bounded_detail(mut detail: String) -> String {
    if detail.len() <= MAX_WORKER_DIAGNOSTIC_BYTES {
        return detail;
    }
    let mut end = MAX_WORKER_DIAGNOSTIC_BYTES;
    while !detail.is_char_boundary(end) {
        end -= 1;
    }
    detail.truncate(end);
    detail.push('…');
    detail
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subprocess_configuration_is_bound_and_rejects_invalid_launches() {
        let adapter = InProcessConfig::default();
        let default = SubprocessConfig::new("worker", adapter.clone());
        let configured = default
            .clone()
            .with_wall_timeout(Duration::from_millis(25))
            .with_max_idle_workers(0);
        assert_eq!(configured.worker_path(), Path::new("worker"));
        assert_eq!(configured.adapter(), &adapter);
        assert_ne!(configured.security_digest(), default.security_digest());
        assert_eq!(
            configured.security_digest_for(&adapter),
            configured.security_digest()
        );

        let zero_timeout =
            SubprocessConfig::new("unused", adapter.clone()).with_wall_timeout(Duration::ZERO);
        let error = SubprocessMonty::new(zero_timeout).unwrap_err();
        assert!(matches!(
            error,
            ExecutionError::Worker(source)
                if source.kind == WorkerFailureKind::Spawn
                    && source.detail.contains("greater than zero")
        ));

        let missing =
            std::env::temp_dir().join(format!("vsh-worker-does-not-exist-{}", std::process::id()));
        let error = SubprocessMonty::new(SubprocessConfig::new(missing, adapter)).unwrap_err();
        assert!(matches!(
            error,
            ExecutionError::Worker(source) if source.kind == WorkerFailureKind::Spawn
        ));
    }

    #[test]
    fn frame_limits_are_kind_specific_and_saturating() {
        let adapter = InProcessConfig::default().with_limits(super::super::ExecutionLimits {
            max_program_bytes: 5,
            max_io_call_bytes: 7,
            max_path_bytes: 11,
            max_output_bytes: 13,
            max_result_bytes: 17,
            max_exception_bytes: 19,
            ..super::super::ExecutionLimits::default()
        });
        let limits = WorkerFrameLimits::new();
        limits.configure(&adapter);

        assert_eq!(limits.for_kind(Some(1)), frame_cap(13));
        assert_eq!(limits.for_kind(Some(3)), frame_cap(29));
        assert_eq!(limits.for_kind(Some(6)), frame_cap(17));
        assert_eq!(limits.for_kind(Some(7)), frame_cap(19));
        assert_eq!(limits.for_kind(None), frame_cap(11));
        assert_eq!(limits.semantic_limit(Some(1)), Some(13));
        assert_eq!(limits.semantic_limit(Some(6)), Some(17));
        assert_eq!(limits.semantic_limit(Some(7)), Some(19));
        assert_eq!(limits.semantic_limit(Some(3)), None);
        assert_eq!(limits.hard(), frame_cap(29));
        assert_eq!(frame_cap(usize::MAX), MAX_FRAME_LEN);
    }

    #[test]
    fn framed_reader_rejects_truncation_and_kind_specific_oversize() {
        let limits = WorkerFrameLimits::new();
        assert!(matches!(
            read_worker_event(&mut [].as_slice(), &limits),
            Ok(None)
        ));
        assert!(matches!(
            read_worker_event(&mut [1_u8].as_slice(), &limits),
            Err(WorkerReadError::Detail(_))
        ));
        assert!(matches!(
            read_worker_event(&mut [2_u8, 0, 0, 0, 0].as_slice(), &limits),
            Err(WorkerReadError::Detail(_))
        ));

        let event = pb::ChildEvent {
            kind: Some(pb::child_event::Kind::Ok(pb::Ok {})),
            ..pb::ChildEvent::default()
        };
        let mut encoded = Vec::new();
        write_frame(&mut encoded, &event).unwrap();
        assert!(matches!(
            read_worker_event(&mut encoded.as_slice(), &limits),
            Ok(Some(decoded)) if matches!(decoded.kind, Some(pb::child_event::Kind::Ok(_)))
        ));

        let adapter = InProcessConfig::default().with_limits(super::super::ExecutionLimits {
            max_output_bytes: 0,
            ..super::super::ExecutionLimits::default()
        });
        limits.configure(&adapter);
        let event = pb::ChildEvent {
            kind: Some(pb::child_event::Kind::Print(pb::Print {
                stream: pb::PrintStream::Stdout.into(),
                text: "x".repeat(FRAME_OVERHEAD_BYTES + 1),
            })),
            ..pb::ChildEvent::default()
        };
        let mut encoded = Vec::new();
        write_frame(&mut encoded, &event).unwrap();
        assert!(matches!(
            read_worker_event(&mut encoded.as_slice(), &limits),
            Err(WorkerReadError::FrameLimit { kind: Some(1), .. })
        ));
    }

    #[test]
    fn protobuf_scanner_and_output_diagnostics_are_strictly_bounded() {
        let mut offset = 0;
        assert!(read_varint(&[], &mut offset).is_err());
        let mut offset = 0;
        assert!(
            read_varint(
                &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 2],
                &mut offset
            )
            .is_err()
        );

        for (bytes, wire_type) in [
            (&[0_u8][..], 0),
            (&[0_u8; 8][..], 1),
            (&[0_u8][..], 2),
            (&[0_u8; 4][..], 5),
        ] {
            let mut offset = 0;
            assert!(skip_protobuf_value(bytes, &mut offset, wire_type).is_ok());
        }
        let mut offset = 0;
        assert!(skip_protobuf_value(&[], &mut offset, 3).is_err());
        let mut offset = 0;
        assert!(skip_protobuf_value(&[8], &mut offset, 1).is_err());

        let mut stdout = String::new();
        let print = pb::Print {
            stream: pb::PrintStream::Stdout.into(),
            text: "ok".to_owned(),
        };
        append_output(&mut stdout, &print, 2).unwrap();
        assert_eq!(stdout, "ok");
        assert!(matches!(
            append_output(&mut stdout, &print, 3),
            Err(ExecutionError::Limit(_))
        ));
        let invalid_stream = pb::Print {
            stream: i32::MAX,
            text: String::new(),
        };
        assert!(matches!(
            append_output(&mut stdout, &invalid_stream, usize::MAX),
            Err(ExecutionError::Worker(source)) if source.kind == WorkerFailureKind::Protocol
        ));

        assert_eq!(bounded_detail("short".to_owned()), "short");
        let bounded = bounded_detail(format!("{}é", "x".repeat(MAX_WORKER_DIAGNOSTIC_BYTES)));
        assert!(bounded.ends_with('…'));
        assert!(bounded.len() <= MAX_WORKER_DIAGNOSTIC_BYTES + '…'.len_utf8());
        assert_eq!(duration_micros(Duration::from_micros(7)), 7);
        assert!(matches!(
            worker_monty_error(
                pb::Error {
                    exception: None,
                },
                1,
            ),
            ExecutionError::Worker(source) if source.kind == WorkerFailureKind::Protocol
        ));
    }

    #[test]
    fn event_classifier_skips_metadata_without_decoding_nested_values() {
        // field 20 varint metadata, followed by field 10's empty length-delimited Ok arm.
        assert_eq!(
            child_event_kind_tag(&[0xa0, 0x01, 0x01, 0x52, 0x00]).unwrap(),
            Some(10)
        );
        assert!(child_event_kind_tag(&[0xa0]).is_err());
        assert!(child_event_kind_tag(&[0x50, 0x00]).is_err());
        assert!(child_event_kind_tag(&[0x52, 0x00, 0x32, 0x00]).is_err());
    }
}
