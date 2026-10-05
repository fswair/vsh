//! Typed Monty-to-VSH execution adapter.
//!
//! The adapter gives Monty no host filesystem mount and answers every typed
//! [`OsFunctionCall`] plus the stable high-level [`MONTY_VSH_TOOL_NAMES`] from a
//! caller-owned [`VirtualFs`]. Both surfaces share one active overlay, policy, budget,
//! and effect ledger. [`InProcessMonty`] is a correctness and embedding harness, not the
//! production isolation boundary: hostile programs must ultimately run in the
//! separately supervised Monty worker described by the VSH architecture.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::time::Duration;

use monty::{MontyRun, RunProgress};
use monty_types::{
    CompileOptions, DictPairs, ExcType, ExtFunctionResult, FileMode, MontyException,
    MontyFileHandle, NameLookupResult, PrintWriter, ResourceLimits, ResourceTracker, StringRepr,
    UnicodeErrorData, UnicodeErrorObject, dir_stat, file_stat, symlink_stat,
    unicode_decode_error_msg, utf8_error_reason,
};
pub use monty_types::{MontyObject, MontyType, OsFunctionCall};
use vsh_policy::{AccessKind, CallPolicy, DeniedAccess};
use vsh_types::{ContentVersion, NodeKind, NodeState, RuntimeConfigDigest, VPath};
use vsh_vfs::{EffectOrigin, VfsError, VirtualFs};

mod search;
mod tools;
mod worker;
mod worker_input;

pub use tools::MONTY_VSH_TOOL_NAMES;
pub use worker::{SubprocessConfig, SubprocessMonty};

use vsh_execution::ExecutionBudget as Budget;
pub use vsh_execution::{
    DEFAULT_VIRTUAL_ROOT, ExecutionLimitExceeded, ExecutionLimits, ExecutionStats,
    VirtualPathError, VirtualRoot, VirtualRootError,
};
use vsh_execution::{FsGateway, GatewayError, OpenMode};
const MAX_PYTHON_RESULT_DEPTH: usize = 200;

/// Host surface whose value-conversion contract must accept an execution result.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ResultCompatibility {
    /// Preserve every valid Monty value for native Rust callers.
    #[default]
    Native,
    /// Reject values the pinned `monty-proto` `PyO3` converter cannot project.
    Python,
    /// Values supported by the agent JSON projection: finite primitives, bytes,
    /// paths, sequences, and dictionaries with string keys only.
    AgentJson,
}

/// A bounded Monty result cannot be represented by the selected host surface.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ResultCompatibilityError {
    /// An agent-facing JSON result would be lossy or impossible to serialize.
    AgentJson {
        /// Stable explanation without guest content.
        reason: String,
    },
    /// The value exceeds the converter's native-stack recursion backstop.
    Depth {
        /// Maximum accepted nesting depth.
        limit: usize,
        /// Nesting depth that first exceeded the limit.
        attempted: usize,
    },
    /// A Monty type object has no faithful host-Python type object.
    TypeObject {
        /// Monty's stable display name for the unsupported type.
        name: String,
    },
}

impl fmt::Display for ResultCompatibilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AgentJson { reason } => write!(
                formatter,
                "result cannot be projected to agent JSON: {reason}"
            ),
            Self::Depth { limit, attempted } => write!(
                formatter,
                "Python result depth exceeds converter limit: {attempted} > {limit}"
            ),
            Self::TypeObject { name } => write!(
                formatter,
                "Monty type object {name:?} has no faithful Python projection"
            ),
        }
    }
}

impl Error for ResultCompatibilityError {}

/// Validate one result before any host mutation that a binding could report as failed.
///
/// Native Rust values are always accepted. Python validation mirrors the exact type-object
/// cases supported by the pinned `monty-proto` converter and its depth backstop.
///
/// # Errors
///
/// Returns [`ResultCompatibilityError`] when Python projection is not total for `value`.
pub fn validate_result_compatibility(
    value: &MontyObject,
    compatibility: ResultCompatibility,
) -> Result<(), ResultCompatibilityError> {
    if compatibility == ResultCompatibility::Native {
        return Ok(());
    }

    let mut pending = vec![(value, 1_usize)];
    while let Some((value, depth)) = pending.pop() {
        if depth > MAX_PYTHON_RESULT_DEPTH {
            return Err(ResultCompatibilityError::Depth {
                limit: MAX_PYTHON_RESULT_DEPTH,
                attempted: depth,
            });
        }
        if compatibility == ResultCompatibility::AgentJson {
            match value {
                MontyObject::None
                | MontyObject::Bool(_)
                | MontyObject::Int(_)
                | MontyObject::String(_)
                | MontyObject::Bytes(_)
                | MontyObject::Path(_)
                | MontyObject::List(_)
                | MontyObject::Tuple(_)
                | MontyObject::NamedTuple { .. } => {}
                MontyObject::Float(number) if number.is_finite() => {}
                MontyObject::Dict(pairs) => {
                    if pairs
                        .into_iter()
                        .any(|(key, _)| !matches!(key, MontyObject::String(_)))
                    {
                        return Err(ResultCompatibilityError::AgentJson {
                            reason: "dictionary keys must be strings; implicit key coercion loses information".into(),
                        });
                    }
                }
                _ => {
                    return Err(ResultCompatibilityError::AgentJson {
                        reason: format!(
                            "unsupported or non-finite {} value; return JSON-compatible data explicitly",
                            value.type_name()
                        ),
                    });
                }
            }
        }
        if let MontyObject::Type(kind) = value
            && !python_type_object_is_supported(kind)
        {
            return Err(ResultCompatibilityError::TypeObject {
                name: kind.to_string(),
            });
        }

        let child_depth = depth.saturating_add(1);
        match value {
            MontyObject::List(values)
            | MontyObject::Tuple(values)
            | MontyObject::Set(values)
            | MontyObject::FrozenSet(values)
            | MontyObject::NamedTuple { values, .. } => {
                pending.extend(values.iter().map(|value| (value, child_depth)));
            }
            MontyObject::Dict(pairs) => {
                for (key, value) in pairs {
                    pending.push((key, child_depth));
                    pending.push((value, child_depth));
                }
            }
            MontyObject::ClassInstance(instance) => {
                for (key, value) in instance.attrs.iter().chain(&instance.class_type.attrs) {
                    pending.push((key, child_depth));
                    pending.push((value, child_depth));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn python_type_object_is_supported(kind: &MontyType) -> bool {
    match kind {
        MontyType::Exception(kind) => !matches!(
            kind,
            ExcType::FrozenInstanceError
                | ExcType::JsonDecodeError
                | ExcType::UnsupportedOperation
                | ExcType::RePatternError
        ),
        MontyType::Ellipsis
        | MontyType::Type
        | MontyType::NoneType
        | MontyType::Bool
        | MontyType::Int
        | MontyType::Float
        | MontyType::Range
        | MontyType::Slice
        | MontyType::Date
        | MontyType::DateTime
        | MontyType::TimeDelta
        | MontyType::TimeZone
        | MontyType::Str
        | MontyType::Bytes
        | MontyType::List
        | MontyType::Deque
        | MontyType::ListIterator
        | MontyType::CallableIterator
        | MontyType::Tuple
        | MontyType::Dict
        | MontyType::Set
        | MontyType::FrozenSet
        | MontyType::TextIOWrapper
        | MontyType::BufferedReader
        | MontyType::BufferedWriter
        | MontyType::BufferedRandom
        | MontyType::SpecialForm
        | MontyType::Path
        | MontyType::Property
        | MontyType::RePattern
        | MontyType::ReMatch
        | MontyType::ItertoolsCount
        | MontyType::ItertoolsRepeat
        | MontyType::Field
        | MontyType::ItertoolsPairwise
        | MontyType::ItertoolsCompress
        | MontyType::ItertoolsIslice
        | MontyType::ItertoolsChain
        | MontyType::ItertoolsCycle => true,
        _ => false,
    }
}

/// Configuration for the process-local Monty correctness harness.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InProcessConfig {
    virtual_root: VirtualRoot,
    environment: BTreeMap<String, String>,
    limits: ExecutionLimits,
    script_name: String,
    call_policy: CallPolicy,
}

impl InProcessConfig {
    /// Construct a config with a caller-selected virtual root and safe defaults.
    #[must_use]
    pub fn new(virtual_root: VirtualRoot) -> Self {
        let mut environment = BTreeMap::new();
        environment.insert("HOME".to_owned(), "/home/vsh".to_owned());
        environment.insert("PWD".to_owned(), virtual_root.as_str().to_owned());
        Self {
            virtual_root,
            environment,
            limits: ExecutionLimits::default(),
            script_name: "<vsh>".to_owned(),
            call_policy: CallPolicy::default(),
        }
    }

    /// Replace all independently enforced execution limits.
    #[must_use]
    pub fn with_limits(mut self, limits: ExecutionLimits) -> Self {
        self.limits = limits;
        self
    }

    /// Replace the complete synthetic environment exposed to Monty.
    #[must_use]
    pub fn with_environment(mut self, environment: BTreeMap<String, String>) -> Self {
        self.environment = environment;
        self
    }

    /// Set the synthetic script name used in Monty tracebacks.
    #[must_use]
    pub fn with_script_name(mut self, script_name: impl Into<String>) -> Self {
        self.script_name = script_name.into();
        self
    }

    /// Replace the complete pre-call path-capability policy.
    #[must_use]
    pub fn with_call_policy(mut self, call_policy: CallPolicy) -> Self {
        self.call_policy = call_policy;
        self
    }

    /// Return the virtual namespace mapping used by this harness.
    #[must_use]
    pub const fn virtual_root(&self) -> &VirtualRoot {
        &self.virtual_root
    }

    /// Return the configured independent host limits.
    #[must_use]
    pub const fn limits(&self) -> ExecutionLimits {
        self.limits
    }

    /// Return the pre-call path-capability policy.
    #[must_use]
    pub const fn call_policy(&self) -> &CallPolicy {
        &self.call_policy
    }

    /// Hash every security-relevant execution setting for transaction binding.
    #[must_use]
    pub fn security_digest(&self) -> RuntimeConfigDigest {
        let mut canonical = Vec::new();
        encode_string("vsh-monty-config-v5", &mut canonical);
        encode_string(env!("CARGO_PKG_VERSION"), &mut canonical);
        encode_string("monty-0.0.22", &mut canonical);
        encode_string("memchr-2.8.3-literal-v1", &mut canonical);
        encode_string(vsh_execution::FS_GATEWAY_VERSION, &mut canonical);
        encode_string(self.virtual_root.as_str(), &mut canonical);
        encode_string(&self.script_name, &mut canonical);
        encode_u64(self.limits.max_program_bytes, &mut canonical);
        canonical.extend_from_slice(&self.limits.max_duration.as_nanos().to_le_bytes());
        encode_u64(self.limits.max_recursion_depth, &mut canonical);
        encode_u64(self.limits.max_memory_bytes, &mut canonical);
        canonical.extend_from_slice(&self.limits.max_os_calls.to_le_bytes());
        canonical.extend_from_slice(&self.limits.max_read_bytes.to_le_bytes());
        canonical.extend_from_slice(&self.limits.max_write_bytes.to_le_bytes());
        encode_u64(self.limits.max_io_call_bytes, &mut canonical);
        encode_u64(self.limits.max_path_bytes, &mut canonical);
        canonical.extend_from_slice(&self.limits.max_directory_entries.to_le_bytes());
        canonical.extend_from_slice(&self.limits.max_evidence_records.to_le_bytes());
        canonical.extend_from_slice(&self.limits.max_evidence_bytes.to_le_bytes());
        encode_u64(self.limits.max_output_bytes, &mut canonical);
        encode_u64(self.limits.max_result_bytes, &mut canonical);
        encode_u64(self.limits.max_exception_bytes, &mut canonical);
        encode_u64(self.environment.len(), &mut canonical);
        for (key, value) in &self.environment {
            encode_string(key, &mut canonical);
            encode_string(value, &mut canonical);
        }
        RuntimeConfigDigest::digest_canonical(&canonical)
    }
}

fn encode_string(value: &str, output: &mut Vec<u8>) {
    encode_u64(value.len(), output);
    output.extend_from_slice(value.as_bytes());
}

fn encode_u64(value: usize, output: &mut Vec<u8>) {
    output.extend_from_slice(&u64::try_from(value).unwrap_or(u64::MAX).to_le_bytes());
}

impl Default for InProcessConfig {
    fn default() -> Self {
        Self::new(VirtualRoot::default())
    }
}

/// Phase in which Monty raised an exception outside sandboxed exception handling.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MontyFailurePhase {
    /// Source parsing or bytecode preparation.
    Compile,
    /// Initial execution or a resumed typed call.
    Runtime,
}

/// Supervised-worker failure category.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum WorkerFailureKind {
    /// The exact configured worker executable could not be validated or spawned.
    Spawn,
    /// A framed request or response could not be transferred.
    Transport,
    /// The child violated the typed Monty protocol.
    Protocol,
    /// The child stopped without a valid turn-ending response.
    Crashed,
    /// The parent wall-clock watchdog expired and terminated the child.
    Timeout,
}

/// Failure reported by the supervised subprocess boundary.
#[derive(Debug)]
pub struct WorkerFailure {
    /// Stable failure category.
    pub kind: WorkerFailureKind,
    /// Bounded diagnostic suitable for logs and mapped errors.
    pub detail: String,
}

impl fmt::Display for WorkerFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Monty worker {:?} failure: {}",
            self.kind, self.detail
        )
    }
}

impl Error for WorkerFailure {}

/// Failure of a Monty execution adapter.
#[derive(Debug)]
#[non_exhaustive]
pub enum ExecutionError {
    /// Terminal nested Bash failure; never resumed as a catchable Monty exception.
    #[cfg(feature = "bash")]
    Bash(Box<vsh_bash::BashError>),
    /// Host cancellation won before commit entry.
    Cancelled,
    /// Monty compilation or runtime failed.
    Monty {
        /// Failure phase.
        phase: MontyFailurePhase,
        /// Exact Monty exception and traceback.
        source: Box<MontyException>,
    },
    /// An independent host-side budget was exceeded and execution was not resumed.
    Limit(Box<ExecutionLimitExceeded>),
    /// VSH snapshot/blob integrity failed; this is never exposed as a catchable Python error.
    InternalVfs(Box<VfsError>),
    /// Monty requested a capability this harness deliberately does not provide.
    UnsupportedSuspension {
        /// Suspension category.
        kind: &'static str,
        /// Function name when Monty supplied one.
        name: Option<String>,
    },
    /// The supervised worker failed outside sandboxed Python semantics.
    Worker(Box<WorkerFailure>),
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(feature = "bash")]
            Self::Bash(source) => write!(formatter, "nested Bash execution failed: {source}"),
            Self::Cancelled => formatter.write_str("execution cancelled"),
            Self::Monty { phase, source } => write!(formatter, "Monty {phase:?} failure: {source}"),
            Self::Limit(source) => write!(formatter, "execution budget failure: {source}"),
            Self::InternalVfs(source) => write!(formatter, "internal VFS failure: {source}"),
            Self::UnsupportedSuspension { kind, name } => match name {
                Some(name) => write!(formatter, "unsupported Monty {kind}: {name}"),
                None => write!(formatter, "unsupported Monty {kind}"),
            },
            Self::Worker(source) => source.fmt(formatter),
        }
    }
}

impl Error for ExecutionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            #[cfg(feature = "bash")]
            Self::Bash(source) => Some(source),
            Self::Monty { source, .. } => Some(source),
            Self::Limit(source) => Some(source),
            Self::InternalVfs(source) => Some(source),
            Self::UnsupportedSuspension { .. } | Self::Cancelled => None,
            Self::Worker(source) => Some(source),
        }
    }
}

/// Successful result of a process-local Monty execution.
#[derive(Debug)]
pub struct ExecutionOutcome {
    /// Final Monty value.
    pub value: MontyObject,
    /// Bounded output captured from `print()`.
    pub stdout: String,
    /// Independent host-side budget counters.
    pub stats: ExecutionStats,
    /// Denials retained even when sandboxed code caught the `PermissionError`.
    pub denied_accesses: Vec<DeniedAccess>,
}

/// Process-local typed adapter used for correctness tests and trusted embedding.
///
/// This type deliberately has no host mount, host environment access, network access,
/// process API, or committer. It does not isolate interpreter crashes; use the planned
/// supervised worker boundary for hostile production execution.
#[derive(Clone, Debug, Default)]
pub struct InProcessMonty {
    config: InProcessConfig,
}

impl InProcessMonty {
    /// Construct the adapter from explicit synthetic-host configuration.
    #[must_use]
    pub const fn new(config: InProcessConfig) -> Self {
        Self { config }
    }

    /// Execute source against a caller-owned virtual transaction.
    ///
    /// # Errors
    ///
    /// Returns [`ExecutionError`] for uncaught Monty exceptions, hard host limits,
    /// integrity failures, or unsupported external-capability suspensions.
    pub fn execute(
        &self,
        code: impl Into<String>,
        filesystem: &mut VirtualFs,
    ) -> Result<ExecutionOutcome, ExecutionError> {
        self.execute_inner(
            code.into(),
            filesystem,
            &vsh_execution::ExecutionCancellation::default(),
            #[cfg(feature = "bash")]
            None,
        )
    }

    /// Execute with an optional host-enabled Bash worker sharing this VFS and budget.
    ///
    /// # Errors
    /// Includes terminal nested frontend failures; failed state must never be committed.
    #[cfg(feature = "bash")]
    pub fn execute_with_bash(
        &self,
        code: impl Into<String>,
        filesystem: &mut VirtualFs,
        cancellation: &vsh_execution::ExecutionCancellation,
        bash: Option<&vsh_bash::SubprocessBash>,
    ) -> Result<ExecutionOutcome, ExecutionError> {
        self.execute_inner(code.into(), filesystem, cancellation, bash)
    }

    #[allow(
        clippy::too_many_lines,
        reason = "keep the fail-closed interpreter suspension state machine in one place"
    )]
    fn execute_inner(
        &self,
        code: String,
        filesystem: &mut VirtualFs,
        cancellation: &vsh_execution::ExecutionCancellation,
        #[cfg(feature = "bash")] bash: Option<&vsh_bash::SubprocessBash>,
    ) -> Result<ExecutionOutcome, ExecutionError> {
        #[cfg(feature = "bash")]
        let bash_enabled = bash.is_some();
        #[cfg(not(feature = "bash"))]
        let bash_enabled = false;
        #[cfg(feature = "bash")]
        let deadline = std::time::Instant::now()
            .checked_add(
                self.config
                    .limits
                    .max_duration
                    .saturating_add(Duration::from_secs(1)),
            )
            .ok_or(ExecutionError::Cancelled)?;
        self.config
            .limits
            .check_program_bytes(code.len())
            .map_err(limit_error)?;
        filesystem.limit_evidence(self.config.limits.evidence_limits());
        filesystem
            .check_evidence()
            .map_err(|source| ExecutionError::InternalVfs(Box::new(source)))?;
        let (input_names, input_values) = tools::inputs(bash_enabled);
        let run = MontyRun::new(
            code,
            &self.config.script_name,
            input_names,
            CompileOptions::default(),
        )
        .map_err(|source| self.monty_error(MontyFailurePhase::Compile, source))?;

        let resource_limits = ResourceLimits::default()
            .max_duration(self.config.limits.max_duration)
            .max_recursion_depth(self.config.limits.max_recursion_depth)
            .max_suspensions(
                usize::try_from(self.config.limits.max_os_calls).unwrap_or(usize::MAX),
            );
        let tracker = ResourceTracker::new(resource_limits);
        let mut stdout = String::new();
        let mut accounted_stdout = 0;
        let mut budget = Budget::new(self.config.limits);
        let mut denied_accesses = Vec::new();
        let mut progress = run
            .start(input_values, tracker, self.print_writer(&mut stdout))
            .map_err(|source| self.monty_error(MontyFailurePhase::Runtime, source))?;

        loop {
            if cancellation.is_cancelled() {
                return Err(ExecutionError::Cancelled);
            }
            progress = match progress {
                RunProgress::Complete(value) => {
                    budget
                        .charge_output(stdout.len().saturating_sub(accounted_stdout))
                        .map_err(limit_error)?;
                    let mut stats = budget.stats();
                    stats.result_bytes =
                        measure_result(&value, self.config.limits.max_result_bytes)
                            .map_err(limit_error)?;
                    return Ok(ExecutionOutcome {
                        value,
                        stdout,
                        stats,
                        denied_accesses,
                    });
                }
                RunProgress::OsCall(call) => {
                    budget.charge_os_call().map_err(limit_error)?;
                    let result =
                        filesystem.with_effect_origin(EffectOrigin::MontyOsCall, |filesystem| {
                            dispatch_call(
                                &call.function_call,
                                filesystem,
                                &self.config,
                                &mut budget,
                            )
                        });
                    let result =
                        call_result(result, filesystem, &mut budget, &mut denied_accesses)?;
                    call.resume(result, self.print_writer(&mut stdout))
                        .map_err(|source| self.monty_error(MontyFailurePhase::Runtime, source))?
                }
                RunProgress::NameLookup(lookup) => lookup
                    .resume(NameLookupResult::Undefined, self.print_writer(&mut stdout))
                    .map_err(|source| self.monty_error(MontyFailurePhase::Runtime, source))?,
                RunProgress::FunctionCall(call) => {
                    budget
                        .charge_output(stdout.len().saturating_sub(accounted_stdout))
                        .map_err(limit_error)?;
                    accounted_stdout = stdout.len();
                    if call.object_id.is_some() || !tools::is_tool(&call.function_name) {
                        return Err(ExecutionError::UnsupportedSuspension {
                            kind: "external function call",
                            name: Some(call.function_name),
                        });
                    }
                    budget.charge_os_call().map_err(limit_error)?;
                    let result =
                        filesystem.with_effect_origin(EffectOrigin::MontyToolCall, |filesystem| {
                            tools::dispatch(
                                &call.function_name,
                                &call.args,
                                &call.kwargs,
                                filesystem,
                                &self.config,
                                &mut budget,
                                #[cfg(feature = "bash")]
                                &mut tools::BashToolContext {
                                    backend: bash,
                                    cancellation: Some(cancellation),
                                    deadline,
                                    denied_accesses: &mut denied_accesses,
                                },
                            )
                        });
                    let result =
                        call_result(result, filesystem, &mut budget, &mut denied_accesses)?;
                    call.resume(result, self.print_writer(&mut stdout))
                        .map_err(|source| self.monty_error(MontyFailurePhase::Runtime, source))?
                }
                RunProgress::ResolveFutures(_) => {
                    return Err(ExecutionError::UnsupportedSuspension {
                        kind: "future resolution",
                        name: None,
                    });
                }
            };
        }
    }

    fn print_writer<'a>(&self, stdout: &'a mut String) -> PrintWriter<'a> {
        PrintWriter::CollectString(stdout, Some(self.config.limits.max_output_bytes))
    }

    fn monty_error(&self, phase: MontyFailurePhase, source: MontyException) -> ExecutionError {
        let attempted = exception_bytes(&source);
        let limit = u64::try_from(self.config.limits.max_exception_bytes).unwrap_or(u64::MAX);
        if attempted > limit {
            limit_error(ExecutionLimitExceeded::ExceptionBytes { limit, attempted })
        } else {
            ExecutionError::Monty {
                phase,
                source: Box::new(source),
            }
        }
    }
}

fn limit_error(source: ExecutionLimitExceeded) -> ExecutionError {
    ExecutionError::Limit(Box::new(source))
}

fn call_result(
    result: Result<MontyObject, CallFailure>,
    filesystem: &mut VirtualFs,
    budget: &mut Budget,
    denied_accesses: &mut Vec<DeniedAccess>,
) -> Result<ExtFunctionResult, ExecutionError> {
    match result {
        #[cfg(feature = "bash")]
        Err(CallFailure::Bash(source)) => Err(ExecutionError::Bash(Box::new(source))),
        Ok(value) => Ok(ExtFunctionResult::Return(value)),
        Err(CallFailure::Python(exception)) => Ok(ExtFunctionResult::Error(exception)),
        Err(CallFailure::Policy(denial)) => {
            budget.record_denial();
            let exception = permission_denied(denial.path.as_str());
            vsh_execution::retain_denial(filesystem, budget.limits(), denied_accesses, denial)
                .map_err(|source| match source {
                    GatewayError::Limit(source) => limit_error(source),
                    GatewayError::Filesystem(source) => ExecutionError::InternalVfs(source),
                    _ => unreachable!("denial retention only checks evidence resources"),
                })?;
            Ok(ExtFunctionResult::Error(exception))
        }
        Err(CallFailure::Limit(source)) => Err(limit_error(source)),
        Err(CallFailure::InternalVfs(source)) => Err(ExecutionError::InternalVfs(Box::new(source))),
    }
}

fn measure_result(value: &MontyObject, limit: usize) -> Result<u64, ExecutionLimitExceeded> {
    let limit = u64::try_from(limit).unwrap_or(u64::MAX);
    let mut used = 0_u64;
    let mut pending = vec![value];
    while let Some(value) = pending.pop() {
        let bytes = u64::try_from(value.host_size()).unwrap_or(u64::MAX);
        let attempted = used.saturating_add(bytes);
        if attempted > limit {
            return Err(ExecutionLimitExceeded::ResultBytes { limit, attempted });
        }
        used = attempted;
        match value {
            MontyObject::List(values)
            | MontyObject::Tuple(values)
            | MontyObject::Set(values)
            | MontyObject::FrozenSet(values)
            | MontyObject::NamedTuple { values, .. } => pending.extend(values),
            MontyObject::Dict(pairs) => {
                for (key, value) in pairs {
                    pending.push(key);
                    pending.push(value);
                }
            }
            MontyObject::ClassInstance(instance) => {
                for (key, value) in instance.attrs.iter().chain(&instance.class_type.attrs) {
                    pending.push(key);
                    pending.push(value);
                }
            }
            _ => {}
        }
    }
    Ok(used)
}

fn exception_bytes(source: &MontyException) -> u64 {
    let mut bytes = u64::try_from(size_of::<MontyException>()).unwrap_or(u64::MAX);
    bytes = bytes.saturating_add(source.message().map_or(0, |message| {
        u64::try_from(message.len()).unwrap_or(u64::MAX)
    }));
    for frame in source.traceback() {
        bytes = bytes
            .saturating_add(u64::try_from(size_of_val(frame)).unwrap_or(u64::MAX))
            .saturating_add(u64::try_from(frame.filename.len()).unwrap_or(u64::MAX))
            .saturating_add(
                frame
                    .frame_name
                    .as_ref()
                    .map_or(0, |name| u64::try_from(name.len()).unwrap_or(u64::MAX)),
            )
            .saturating_add(
                frame
                    .preview_line
                    .as_ref()
                    .map_or(0, |line| u64::try_from(line.len()).unwrap_or(u64::MAX)),
            );
    }
    if let Some(data) = source.data().unicode() {
        bytes = bytes
            .saturating_add(u64::try_from(data.encoding.len()).unwrap_or(u64::MAX))
            .saturating_add(u64::try_from(data.reason.len()).unwrap_or(u64::MAX))
            .saturating_add(match &data.object {
                UnicodeErrorObject::Bytes(value) => u64::try_from(value.len()).unwrap_or(u64::MAX),
                UnicodeErrorObject::Str(value) => u64::try_from(value.len()).unwrap_or(u64::MAX),
            });
    }
    if let Some(data) = source.data().json() {
        bytes = bytes
            .saturating_add(u64::try_from(data.msg.len()).unwrap_or(u64::MAX))
            .saturating_add(
                data.doc
                    .as_ref()
                    .map_or(0, |doc| u64::try_from(doc.len()).unwrap_or(u64::MAX)),
            );
    }
    bytes
}

enum CallFailure {
    #[cfg(feature = "bash")]
    Bash(vsh_bash::BashError),
    Python(MontyException),
    Policy(DeniedAccess),
    Limit(ExecutionLimitExceeded),
    InternalVfs(VfsError),
}

impl From<ExecutionLimitExceeded> for CallFailure {
    fn from(source: ExecutionLimitExceeded) -> Self {
        Self::Limit(source)
    }
}

impl From<GatewayError> for CallFailure {
    fn from(source: GatewayError) -> Self {
        let raw = match &source {
            GatewayError::Policy(source) => source.path.to_string(),
            GatewayError::Filesystem(source) => match source.as_ref() {
                VfsError::NotFound { path }
                | VfsError::AlreadyExists { path }
                | VfsError::NotDirectory { path, .. }
                | VfsError::NotFile { path, .. }
                | VfsError::NotSymlink { path, .. }
                | VfsError::IsDirectory { path }
                | VfsError::DirectoryNotEmpty { path } => path.to_string(),
                VfsError::InvalidRename { from, .. }
                | VfsError::RenameTypeMismatch { from, .. } => from.to_string(),
                _ => "<virtual filesystem>".to_owned(),
            },
            _ => "<virtual filesystem>".to_owned(),
        };
        classify_gateway(source, &raw)
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "keeping the exhaustive typed Monty boundary in one match makes new upstream variants fail compilation"
)]
fn dispatch_call(
    call: &OsFunctionCall,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    match call {
        OsFunctionCall::Exists(path) => {
            bool_query(call, path.as_str(), filesystem, config, budget, |state| {
                state.is_some()
            })
        }
        OsFunctionCall::IsFile(path) => {
            bool_query(call, path.as_str(), filesystem, config, budget, |state| {
                state.is_some_and(|state| state.kind() == NodeKind::File)
            })
        }
        OsFunctionCall::IsDir(path) => {
            bool_query(call, path.as_str(), filesystem, config, budget, |state| {
                state.is_some_and(|state| state.kind() == NodeKind::Directory)
            })
        }
        OsFunctionCall::IsSymlink(path) => {
            bool_query(call, path.as_str(), filesystem, config, budget, |state| {
                state.is_some_and(|state| state.kind() == NodeKind::Symlink)
            })
        }
        OsFunctionCall::ReadText(path) => {
            read_text(call, path.as_str(), filesystem, config, budget)
        }
        OsFunctionCall::ReadBytes(path) => {
            read_bytes(call, path.as_str(), filesystem, config, budget)
        }
        OsFunctionCall::Stat(path) => stat(call, path.as_str(), filesystem, config, budget),
        OsFunctionCall::Iterdir(path) => {
            read_directory(call, path.as_str(), filesystem, config, budget)
        }
        OsFunctionCall::Resolve(path) | OsFunctionCall::Absolute(path) => {
            absolute_path(call, path.as_str(), config)
        }
        OsFunctionCall::WriteText(args) => {
            write_bytes(
                call,
                args.path.as_str(),
                args.data.as_bytes(),
                filesystem,
                config,
                budget,
            )?;
            Ok(MontyObject::Int(
                i64::try_from(args.data.chars().count()).unwrap_or(i64::MAX),
            ))
        }
        OsFunctionCall::WriteBytes(args) => {
            write_bytes(
                call,
                args.path.as_str(),
                &args.data,
                filesystem,
                config,
                budget,
            )?;
            Ok(MontyObject::Int(
                i64::try_from(args.data.len()).unwrap_or(i64::MAX),
            ))
        }
        OsFunctionCall::AppendText(args) => {
            append_bytes(
                call,
                args.path.as_str(),
                args.data.as_bytes(),
                filesystem,
                config,
                budget,
            )?;
            Ok(MontyObject::Int(
                i64::try_from(args.data.chars().count()).unwrap_or(i64::MAX),
            ))
        }
        OsFunctionCall::AppendBytes(args) => {
            append_bytes(
                call,
                args.path.as_str(),
                &args.data,
                filesystem,
                config,
                budget,
            )?;
            Ok(MontyObject::Int(
                i64::try_from(args.data.len()).unwrap_or(i64::MAX),
            ))
        }
        OsFunctionCall::Open(args) => open_file(
            call,
            args.path.as_str(),
            args.mode,
            filesystem,
            config,
            budget,
        ),
        OsFunctionCall::Mkdir(args) => mkdir(
            call,
            args.path.as_str(),
            args.parents,
            args.exist_ok,
            filesystem,
            config,
            budget,
        ),
        OsFunctionCall::Unlink(path) => {
            let mapped = map_call_path(call, path.as_str(), false, config)?;
            gateway(filesystem, config, budget)
                .unlink(&mapped)
                .map_err(|source| classify_gateway(source, path.as_str()))?;
            Ok(MontyObject::None)
        }
        OsFunctionCall::Rmdir(path) => {
            let mapped = map_call_path(call, path.as_str(), false, config)?;
            gateway(filesystem, config, budget)
                .rmdir(&mapped)
                .map_err(|source| classify_gateway(source, path.as_str()))?;
            Ok(MontyObject::None)
        }
        OsFunctionCall::Rename(args) => {
            let source = map_authorized_path(
                call,
                args.src.as_str(),
                false,
                config,
                &[AccessKind::RenameSource],
            )?;
            let destination = map_authorized_path(
                call,
                args.dst.as_str(),
                true,
                config,
                &[AccessKind::RenameDestination],
            )?;
            gateway(filesystem, config, budget)
                .rename(&source, &destination)
                .map_err(|source| classify_gateway(source, args.src.as_str()))?;
            Ok(MontyObject::None)
        }
        OsFunctionCall::Getenv(args) => Ok(config.environment.get(&args.key).map_or_else(
            || args.default.clone(),
            |value| MontyObject::String(value.clone()),
        )),
        OsFunctionCall::GetEnviron => {
            let pairs = config
                .environment
                .iter()
                .map(|(key, value)| {
                    (
                        MontyObject::String(key.clone()),
                        MontyObject::String(value.clone()),
                    )
                })
                .collect::<Vec<_>>();
            Ok(MontyObject::Dict(DictPairs::from(pairs)))
        }
        OsFunctionCall::DateToday | OsFunctionCall::DateTimeNow(_) => {
            Err(CallFailure::Python(call.on_no_handler()))
        }
    }
}

fn bool_query(
    call: &OsFunctionCall,
    raw: &str,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
    predicate: impl FnOnce(Option<NodeState>) -> bool,
) -> Result<MontyObject, CallFailure> {
    check_path_bytes(raw, config)?;
    let Ok(path) = config.virtual_root.map_path(raw) else {
        return Ok(MontyObject::Bool(false));
    };
    let state = match gateway(filesystem, config, budget).metadata(&path) {
        Ok(state) => Some(state),
        Err(GatewayError::Filesystem(source)) if matches!(*source, VfsError::NotFound { .. }) => {
            None
        }
        Err(source) => return Err(classify_gateway(source, raw)),
    };
    let _ = call;
    Ok(MontyObject::Bool(predicate(state)))
}

fn read_text(
    call: &OsFunctionCall,
    raw: &str,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let bytes = read_file(call, raw, filesystem, config, budget)?;
    match String::from_utf8(bytes) {
        Ok(text) => Ok(MontyObject::String(text)),
        Err(error) => {
            let utf8 = error.utf8_error();
            let start = utf8.valid_up_to();
            let end = utf8
                .error_len()
                .map_or(error.as_bytes().len(), |length| start + length);
            let first_byte = error.as_bytes()[start];
            let reason = utf8_error_reason(first_byte, utf8.error_len());
            let data = UnicodeErrorData::decode("utf-8", error.as_bytes(), start, end, reason);
            Err(CallFailure::Python(
                MontyException::new(
                    ExcType::UnicodeDecodeError,
                    Some(unicode_decode_error_msg(
                        "utf-8", first_byte, start, end, reason,
                    )),
                )
                .with_data(data),
            ))
        }
    }
}

fn read_bytes(
    call: &OsFunctionCall,
    raw: &str,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    read_file(call, raw, filesystem, config, budget).map(MontyObject::Bytes)
}

fn read_file(
    call: &OsFunctionCall,
    raw: &str,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<Vec<u8>, CallFailure> {
    let path = map_call_path(call, raw, false, config)?;
    gateway(filesystem, config, budget)
        .read(&path)
        .map_err(|source| classify_gateway(source, raw))
}

fn write_bytes(
    call: &OsFunctionCall,
    raw: &str,
    bytes: &[u8],
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<(), CallFailure> {
    let path = map_call_path(call, raw, false, config)?;
    gateway(filesystem, config, budget)
        .write(&path, bytes)
        .map_err(|source| classify_gateway(source, raw))
}

fn append_bytes(
    call: &OsFunctionCall,
    raw: &str,
    bytes: &[u8],
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<(), CallFailure> {
    let path = map_call_path(call, raw, false, config)?;
    gateway(filesystem, config, budget)
        .append(&path, bytes)
        .map_err(|source| classify_gateway(source, raw))
}

fn read_directory(
    call: &OsFunctionCall,
    raw: &str,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let path = map_call_path(call, raw, false, config)?;
    let children = gateway(filesystem, config, budget)
        .read_dir(&path)
        .map_err(|source| classify_gateway(source, raw))?;
    Ok(MontyObject::List(
        children
            .iter()
            .map(|child| MontyObject::Path(config.virtual_root.present(child)))
            .collect(),
    ))
}

fn stat(
    call: &OsFunctionCall,
    raw: &str,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let path = map_call_path(call, raw, false, config)?;
    let state = gateway(filesystem, config, budget)
        .metadata(&path)
        .map_err(|source| classify_gateway(source, raw))?;
    let mtime = match state.content() {
        Some(ContentVersion::Stamp(stamp)) => u64::try_from(stamp.mtime_ns)
            .map(Duration::from_nanos)
            .map_or(0.0, |duration| duration.as_secs_f64()),
        _ => 0.0,
    };
    let mode = i64::from(state.mode());
    let size = i64::try_from(state.size()).unwrap_or(i64::MAX);
    match state.kind() {
        NodeKind::File => Ok(file_stat(mode, size, mtime)),
        NodeKind::Directory => Ok(dir_stat(mode, mtime)),
        NodeKind::Symlink => Ok(symlink_stat(mode, mtime)),
    }
}

fn absolute_path(
    call: &OsFunctionCall,
    raw: &str,
    config: &InProcessConfig,
) -> Result<MontyObject, CallFailure> {
    let path = map_call_path(call, raw, false, config)?;
    Ok(MontyObject::Path(config.virtual_root.present(&path)))
}

fn open_file(
    call: &OsFunctionCall,
    raw: &str,
    mode: FileMode,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let action = match mode {
        FileMode::Read(_) => OpenMode::Read,
        FileMode::ReadUpdate(_) => OpenMode::ReadUpdate,
        FileMode::Write(_) => OpenMode::Write,
        FileMode::WriteUpdate(_) => OpenMode::WriteUpdate,
        FileMode::Append(_) => OpenMode::Append,
        FileMode::AppendUpdate(_) => OpenMode::AppendUpdate,
    };
    let path = map_call_path(call, raw, false, config)?;
    gateway(filesystem, config, budget)
        .prepare_open(&path, action)
        .map_err(|source| classify_gateway(source, raw))?;
    Ok(MontyObject::FileHandle(MontyFileHandle {
        path: config.virtual_root.present(&path),
        mode,
        position: 0,
    }))
}

fn mkdir(
    call: &OsFunctionCall,
    raw: &str,
    parents: bool,
    exist_ok: bool,
    filesystem: &mut VirtualFs,
    config: &InProcessConfig,
    budget: &mut Budget,
) -> Result<MontyObject, CallFailure> {
    let path = map_call_path(call, raw, false, config)?;
    gateway(filesystem, config, budget)
        .mkdir_options(&path, 0o755, parents, exist_ok)
        .map_err(|source| classify_gateway(source, raw))?;
    Ok(MontyObject::None)
}

fn map_call_path(
    call: &OsFunctionCall,
    raw: &str,
    destination: bool,
    config: &InProcessConfig,
) -> Result<VPath, CallFailure> {
    check_path_bytes(raw, config)?;
    config.virtual_root.map_path(raw).map_err(|source| {
        if source == VirtualPathError::NulByte {
            CallFailure::Python(MontyException::new(
                ExcType::ValueError,
                Some(call.embedded_null_message(destination).to_owned()),
            ))
        } else {
            CallFailure::Python(permission_denied(raw))
        }
    })
}

fn check_path_bytes(raw: &str, config: &InProcessConfig) -> Result<(), CallFailure> {
    vsh_execution::check_path_bytes(raw, config.limits).map_err(CallFailure::Limit)
}

fn map_authorized_path(
    call: &OsFunctionCall,
    raw: &str,
    destination: bool,
    config: &InProcessConfig,
    accesses: &[AccessKind],
) -> Result<VPath, CallFailure> {
    let path = map_call_path(call, raw, destination, config)?;
    authorize_path(config, &path, accesses)?;
    Ok(path)
}

fn authorize_path(
    config: &InProcessConfig,
    path: &VPath,
    accesses: &[AccessKind],
) -> Result<(), CallFailure> {
    vsh_execution::authorize_path(&config.call_policy, path, accesses).map_err(CallFailure::Policy)
}

fn gateway<'a>(
    filesystem: &'a mut VirtualFs,
    config: &'a InProcessConfig,
    budget: &'a mut Budget,
) -> FsGateway<'a> {
    let origin = filesystem.effect_origin();
    FsGateway::new(filesystem, &config.call_policy, budget, origin)
}

fn classify_gateway(source: GatewayError, raw: &str) -> CallFailure {
    match source {
        source @ GatewayError::InvalidOperation { .. } => CallFailure::Python(MontyException::new(
            ExcType::ValueError,
            Some(format!("vsh_copy() {source}")),
        )),
        GatewayError::Policy(source) => CallFailure::Policy(*source),
        GatewayError::Limit(source) => CallFailure::Limit(source),
        GatewayError::Filesystem(source) => classify_vfs(*source, raw),
    }
}

fn classify_vfs(source: VfsError, raw: &str) -> CallFailure {
    let exception = match source {
        VfsError::NotFound { .. } => file_not_found(raw),
        VfsError::AlreadyExists { .. } => already_exists_exception(raw),
        VfsError::NotDirectory { .. } => not_directory_exception(raw),
        VfsError::NotFile {
            actual: NodeKind::Directory,
            ..
        }
        | VfsError::IsDirectory { .. } => is_directory_exception(raw),
        VfsError::NotFile { .. } | VfsError::NotSymlink { .. } | VfsError::RootMutation => {
            permission_denied(raw)
        }
        VfsError::DirectoryNotEmpty { .. } => MontyException::new(
            ExcType::OSError,
            Some(format!(
                "[Errno 39] Directory not empty: {}",
                StringRepr(raw)
            )),
        ),
        VfsError::InvalidRename { .. } | VfsError::RenameTypeMismatch { .. } => {
            MontyException::new(
                ExcType::OSError,
                Some(format!("[Errno 22] Invalid argument: {}", StringRepr(raw))),
            )
        }
        internal @ (VfsError::Snapshot(_) | VfsError::Store(_) | VfsError::Path(_)) => {
            return CallFailure::InternalVfs(internal);
        }
        internal => return CallFailure::InternalVfs(internal),
    };
    CallFailure::Python(exception)
}

fn file_not_found(raw: &str) -> MontyException {
    MontyException::new(
        ExcType::FileNotFoundError,
        Some(format!(
            "[Errno 2] No such file or directory: {}",
            StringRepr(raw)
        )),
    )
}

fn already_exists_exception(raw: &str) -> MontyException {
    MontyException::new(
        ExcType::FileExistsError,
        Some(format!("[Errno 17] File exists: {}", StringRepr(raw))),
    )
}

fn is_directory_exception(raw: &str) -> MontyException {
    MontyException::new(
        ExcType::IsADirectoryError,
        Some(format!("[Errno 21] Is a directory: {}", StringRepr(raw))),
    )
}

fn not_directory(raw: &str) -> CallFailure {
    CallFailure::Python(not_directory_exception(raw))
}

fn not_directory_exception(raw: &str) -> MontyException {
    MontyException::new(
        ExcType::NotADirectoryError,
        Some(format!("[Errno 20] Not a directory: {}", StringRepr(raw))),
    )
}

fn permission_denied(raw: &str) -> MontyException {
    MontyException::new(
        ExcType::PermissionError,
        Some(format!("[Errno 13] Permission denied: {}", StringRepr(raw))),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    use monty_types::ExcType;
    use vsh_policy::{CallPolicy, DenyReason, PolicyDecision, PolicyInput, TransactionPolicy};
    use vsh_store::BlobStore;
    use vsh_types::{DiffKind, VPath};
    use vsh_vfs::{EffectOrigin, SnapshotBuilder, VfsError, VirtualFs};

    use super::{
        ExecutionError, ExecutionLimitExceeded, ExecutionLimits, InProcessConfig, InProcessMonty,
        MontyObject, MontyType, ResultCompatibility, ResultCompatibilityError, VirtualPathError,
        VirtualRoot, VirtualRootError, WorkerFailure, WorkerFailureKind,
        validate_result_compatibility,
    };

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("vsh-monty-test-{}-{sequence}", std::process::id()));
            fs::create_dir(&path).expect("test directory should be unique");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn filesystem(files: &[(&str, &[u8])]) -> (TestDirectory, VirtualFs) {
        let directory = TestDirectory::new();
        let store = BlobStore::open(directory.path()).expect("blob store should open");
        let mut builder = SnapshotBuilder::new(store);
        for (path, bytes) in files {
            builder
                .add_file(
                    VPath::parse(path).expect("test path should parse"),
                    bytes,
                    0o644,
                )
                .expect("test file should be added");
        }
        let snapshot = builder.build().expect("snapshot should build");
        (directory, VirtualFs::new(snapshot))
    }

    #[test]
    fn virtual_root_maps_only_its_absolute_namespace() {
        let root = VirtualRoot::new("/workspace/").expect("root should normalize");
        assert_eq!(root.as_str(), "/workspace");
        assert_eq!(
            root.map_path("/workspace/src/../README.md")
                .expect("path should map"),
            VPath::parse("README.md").unwrap()
        );
        assert_eq!(
            root.map_path("/etc/passwd"),
            Err(VirtualPathError::OutsideRoot)
        );
        assert_eq!(
            root.map_path("/workspace/../../etc/passwd"),
            Err(VirtualPathError::EscapesAbsoluteRoot)
        );
    }

    #[test]
    fn monty_program_produces_exact_virtual_diff() {
        let (_directory, mut filesystem) = filesystem(&[("input.txt", b"hello\n")]);
        let outcome = InProcessMonty::default()
            .execute(
                r"
from pathlib import Path
source = Path('/workspace/input.txt').read_text()
Path('/workspace/out').mkdir()
Path('/workspace/out/result.txt').write_text(source.upper())
Path('/workspace/input.txt').rename('/workspace/archive.txt')
len(source)
",
                &mut filesystem,
            )
            .expect("program should execute");

        assert_eq!(outcome.value, MontyObject::Int(6));
        assert_eq!(outcome.stats.os_calls, 4);
        assert_eq!(outcome.stats.read_bytes, 6);
        assert_eq!(outcome.stats.write_bytes, 6);
        assert!(
            filesystem
                .effects()
                .iter()
                .all(|event| event.origin == EffectOrigin::MontyOsCall)
        );

        let diff = filesystem
            .canonical_diff()
            .expect("diff should be canonical");
        let changes = diff
            .entries()
            .iter()
            .map(|entry| (entry.path.as_str(), entry.kind))
            .collect::<Vec<_>>();
        assert_eq!(
            changes,
            vec![
                ("archive.txt", DiffKind::Create),
                ("input.txt", DiffKind::Delete),
                ("out", DiffKind::Create),
                ("out/result.txt", DiffKind::Create),
            ]
        );
        assert_eq!(
            filesystem
                .read(&VPath::parse("out/result.txt").unwrap())
                .unwrap(),
            b"HELLO\n"
        );
    }

    #[test]
    fn monty_tools_and_pathlib_share_one_active_virtual_filesystem() {
        let (_directory, mut filesystem) = filesystem(&[]);
        let outcome = InProcessMonty::default()
            .execute(
                r"
from pathlib import Path
vsh_mkdir('/workspace/out')
vsh_write('/workspace/out/tool.txt', 'hello')
seen_by_pathlib = Path('/workspace/out/tool.txt').read_text()
Path('/workspace/out/pathlib.txt').write_text(seen_by_pathlib.upper())
seen_by_tool = vsh_read('/workspace/out/pathlib.txt')
(seen_by_pathlib, seen_by_tool, len(vsh_list('/workspace/out')))
",
                &mut filesystem,
            )
            .expect("VSH functions and pathlib should interoperate");

        assert_eq!(
            outcome.value,
            MontyObject::Tuple(vec![
                MontyObject::String("hello".to_owned()),
                MontyObject::String("HELLO".to_owned()),
                MontyObject::Int(2),
            ])
        );
        assert_eq!(
            filesystem
                .read(&VPath::parse("out/pathlib.txt").unwrap())
                .unwrap(),
            b"HELLO"
        );
        assert!(
            filesystem
                .effects()
                .iter()
                .any(|event| event.origin == EffectOrigin::MontyToolCall)
        );
        assert!(
            filesystem
                .effects()
                .iter()
                .any(|event| event.origin == EffectOrigin::MontyOsCall)
        );
    }

    #[test]
    fn monty_tools_copy_glob_search_patch_move_and_remove_virtual_state() {
        let (_directory, mut filesystem) = filesystem(&[]);
        let outcome = InProcessMonty::default()
            .execute(
                r"
vsh_mkdir('/workspace/src/nested')
vsh_write('/workspace/src/a.txt', 'Needle one\n')
vsh_write('/workspace/src/nested/b.txt', 'needle two\n')
vsh_copy('/workspace/src', '/workspace/copied', recursive=True)
paths = vsh_glob('**/*.txt', path='/workspace/copied')
hits = vsh_search('needle', path='/workspace/copied', case_sensitive=False)
changed = vsh_patch('/workspace/copied/a.txt', 'Needle', 'Found')
vsh_move('/workspace/copied/a.txt', '/workspace/copied/renamed.txt')
vsh_remove('/workspace/copied/nested', recursive=True)
(len(paths), len(hits), changed, vsh_read('/workspace/copied/renamed.txt'), len(vsh_list('/workspace/copied')))
",
                &mut filesystem,
            )
            .expect("high-level VSH functions should compose on one overlay");

        assert_eq!(
            outcome.value,
            MontyObject::Tuple(vec![
                MontyObject::Int(2),
                MontyObject::Int(2),
                MontyObject::Int(1),
                MontyObject::String("Found one\n".to_owned()),
                MontyObject::Int(1),
            ])
        );
        assert_eq!(
            filesystem
                .read(&VPath::parse("copied/renamed.txt").unwrap())
                .unwrap(),
            b"Found one\n"
        );
        assert!(
            !filesystem
                .exists(&VPath::parse("copied/nested").unwrap())
                .unwrap()
        );
    }

    #[test]
    fn bounded_glob_and_search_stop_walking_after_enough_results() {
        let (_directory, mut filesystem) = filesystem(&[
            ("a.txt", b"needle"),
            ("b.txt", b"needle"),
            ("c.txt", b"needle"),
        ]);
        let outcome = InProcessMonty::default()
            .execute(
                r"
paths = vsh_glob('*.txt', max_results=1)
hits = vsh_search('needle', max_results=1)
(paths, hits[0]['path'])
",
                &mut filesystem,
            )
            .expect("bounded discovery should complete");

        assert_eq!(
            outcome.value,
            MontyObject::Tuple(vec![
                MontyObject::List(vec![MontyObject::Path("/workspace/a.txt".to_owned(),)]),
                MontyObject::Path("/workspace/a.txt".to_owned()),
            ])
        );
        assert_eq!(outcome.stats.read_bytes, 6);
    }

    #[test]
    fn zero_result_discovery_validates_the_root_without_walking_it() {
        let (_directory, mut filesystem) = filesystem(&[("a.txt", b"needle")]);
        let outcome = InProcessMonty::default()
            .execute(
                r"
(vsh_glob('*.txt', max_results=0), vsh_search('needle', max_results=0))
",
                &mut filesystem,
            )
            .expect("zero-result discovery should remain valid and bounded");

        assert_eq!(
            outcome.value,
            MontyObject::Tuple(vec![MontyObject::List(vec![]), MontyObject::List(vec![])])
        );
        assert_eq!(outcome.stats.directory_entries, 0);
        assert_eq!(outcome.stats.read_bytes, 0);
    }

    #[test]
    fn absolute_host_path_never_falls_back_to_host() {
        let (directory, mut filesystem) = filesystem(&[]);
        let host_file = directory.path().join("host-secret.txt");
        fs::write(&host_file, b"secret").expect("host sentinel should be written");
        let code = format!(
            "from pathlib import Path\nPath({:?}).exists()",
            host_file.to_string_lossy()
        );
        let outcome = InProcessMonty::default()
            .execute(code, &mut filesystem)
            .expect("existence check should safely complete");
        assert_eq!(outcome.value, MontyObject::Bool(false));
        assert_eq!(outcome.stats.read_bytes, 0);
    }

    #[test]
    fn traversal_read_is_denied_without_resuming_host_access() {
        let (_directory, mut filesystem) = filesystem(&[]);
        let error = InProcessMonty::default()
            .execute(
                "from pathlib import Path\nPath('/workspace/../../etc/passwd').read_text()",
                &mut filesystem,
            )
            .expect_err("traversal must fail");
        let ExecutionError::Monty { source, .. } = error else {
            panic!("expected a Monty exception")
        };
        assert_eq!(source.exc_type(), ExcType::PermissionError);
    }

    #[test]
    fn active_evidence_limits_abort_caught_reads_and_denials_before_complete_output() {
        for (code, limits) in [
            (
                "from pathlib import Path\nfor i in range(4):\n    try:\n        Path('/workspace/missing').stat()\n    except OSError:\n        pass\n'done'",
                ExecutionLimits {
                    max_evidence_records: 3,
                    ..ExecutionLimits::default()
                },
            ),
            (
                "from pathlib import Path\nfor i in range(4):\n    try:\n        Path('/workspace/.env').read_text()\n    except PermissionError:\n        pass\n'done'",
                ExecutionLimits {
                    max_evidence_records: 2,
                    ..ExecutionLimits::default()
                },
            ),
            (
                "from pathlib import Path\nPath('/workspace/missing').exists()",
                ExecutionLimits {
                    max_evidence_bytes: 0,
                    ..ExecutionLimits::default()
                },
            ),
        ] {
            let (_directory, mut filesystem) = filesystem(&[]);
            let engine = InProcessMonty::new(InProcessConfig::default().with_limits(limits));
            assert!(
                matches!(engine.execute(code, &mut filesystem), Err(ExecutionError::Limit(source)) if matches!(*source, ExecutionLimitExceeded::EvidenceRecords { .. } | ExecutionLimitExceeded::EvidenceBytes { .. }))
            );
            assert!(filesystem.canonical_diff().is_err());
            assert!(engine.execute("42", &mut filesystem).is_err());
        }
    }

    #[test]
    fn independent_os_call_limit_is_hard() {
        let (_directory, mut filesystem) = filesystem(&[]);
        let limits = ExecutionLimits {
            max_os_calls: 2,
            ..ExecutionLimits::default()
        };
        let engine = InProcessMonty::new(InProcessConfig::default().with_limits(limits));
        let error = engine
            .execute(
                r"
from pathlib import Path
Path('/workspace/a').exists()
Path('/workspace/b').exists()
Path('/workspace/c').exists()
",
                &mut filesystem,
            )
            .expect_err("third call must be rejected");
        assert!(matches!(
            error,
            ExecutionError::Limit(source)
                if *source == ExecutionLimitExceeded::OsCalls { limit: 2, attempted: 3 }
        ));
    }

    #[test]
    fn high_level_vsh_tools_share_the_hard_os_call_budget() {
        let (_directory, mut filesystem) = filesystem(&[]);
        let limits = ExecutionLimits {
            max_os_calls: 2,
            ..ExecutionLimits::default()
        };
        let engine = InProcessMonty::new(InProcessConfig::default().with_limits(limits));
        let error = engine
            .execute(
                r"
vsh_write('/workspace/a.txt', 'a')
vsh_read('/workspace/a.txt')
vsh_list('/workspace')
",
                &mut filesystem,
            )
            .expect_err("the third high-level VSH call must be rejected");
        assert!(matches!(
            error,
            ExecutionError::Limit(source)
                if *source == ExecutionLimitExceeded::OsCalls { limit: 2, attempted: 3 }
        ));
    }

    #[test]
    fn per_call_payload_and_path_limits_stop_before_vfs_mutation() {
        let (_directory, mut filesystem) = filesystem(&[("input.txt", b"four")]);
        let limits = ExecutionLimits {
            max_io_call_bytes: 3,
            ..ExecutionLimits::default()
        };
        let error = InProcessMonty::new(InProcessConfig::default().with_limits(limits))
            .execute(
                "from pathlib import Path\nPath('/workspace/input.txt').read_bytes()",
                &mut filesystem,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            ExecutionError::Limit(source)
                if *source == ExecutionLimitExceeded::ReadCallBytes {
                    limit: 3,
                    attempted: 4,
                }
        ));
        let error = InProcessMonty::new(InProcessConfig::default().with_limits(limits))
            .execute(
                "from pathlib import Path\nPath('/workspace/output.txt').write_text('four')",
                &mut filesystem,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            ExecutionError::Limit(source)
                if *source == ExecutionLimitExceeded::WriteCallBytes {
                    limit: 3,
                    attempted: 4,
                }
        ));

        let limits = ExecutionLimits {
            max_path_bytes: 8,
            ..ExecutionLimits::default()
        };
        let error = InProcessMonty::new(InProcessConfig::default().with_limits(limits))
            .execute(
                "from pathlib import Path\nPath('/workspace/too-long').write_text('x')",
                &mut filesystem,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            ExecutionError::Limit(source)
                if matches!(*source, ExecutionLimitExceeded::PathBytes { limit: 8, .. })
        ));
        assert!(filesystem.canonical_diff().unwrap().is_empty());
    }

    #[test]
    fn program_result_and_exception_outputs_have_independent_hard_caps() {
        let (_directory, mut filesystem) = filesystem(&[]);
        let program_limits = ExecutionLimits {
            max_program_bytes: 4,
            ..ExecutionLimits::default()
        };
        let error = InProcessMonty::new(InProcessConfig::default().with_limits(program_limits))
            .execute("'too long'", &mut filesystem)
            .unwrap_err();
        assert!(matches!(
            error,
            ExecutionError::Limit(source)
                if matches!(*source, ExecutionLimitExceeded::ProgramBytes { limit: 4, .. })
        ));

        let result_limits = ExecutionLimits {
            max_result_bytes: 128,
            ..ExecutionLimits::default()
        };
        let error = InProcessMonty::new(InProcessConfig::default().with_limits(result_limits))
            .execute("'x' * 1_000", &mut filesystem)
            .unwrap_err();
        assert!(matches!(
            error,
            ExecutionError::Limit(source)
                if matches!(*source, ExecutionLimitExceeded::ResultBytes { limit: 128, .. })
        ));

        let exception_limits = ExecutionLimits {
            max_exception_bytes: 128,
            ..ExecutionLimits::default()
        };
        let error = InProcessMonty::new(InProcessConfig::default().with_limits(exception_limits))
            .execute("raise ValueError('x' * 1_000)", &mut filesystem)
            .unwrap_err();
        assert!(matches!(
            error,
            ExecutionError::Limit(source)
                if matches!(*source, ExecutionLimitExceeded::ExceptionBytes { limit: 128, .. })
        ));
    }

    #[test]
    fn security_digest_changes_with_synthetic_environment_and_limits() {
        let base = InProcessConfig::default();
        let mut environment = BTreeMap::new();
        environment.insert("PWD".to_owned(), "/workspace".to_owned());
        let changed_environment = base.clone().with_environment(environment);
        let changed_limit = base.clone().with_limits(ExecutionLimits {
            max_os_calls: base.limits().max_os_calls - 1,
            ..base.limits()
        });

        assert_ne!(
            base.security_digest(),
            changed_environment.security_digest()
        );
        assert_ne!(base.security_digest(), changed_limit.security_digest());
        for limits in [
            ExecutionLimits {
                max_evidence_records: base.limits().max_evidence_records - 1,
                ..base.limits()
            },
            ExecutionLimits {
                max_evidence_bytes: base.limits().max_evidence_bytes - 1,
                ..base.limits()
            },
        ] {
            assert_ne!(
                base.security_digest(),
                base.clone().with_limits(limits).security_digest()
            );
        }
        assert_eq!(
            base.security_digest(),
            InProcessConfig::default().security_digest()
        );
    }

    #[test]
    fn environment_is_synthetic_and_secret_free() {
        let (_directory, mut filesystem) = filesystem(&[]);
        let outcome = InProcessMonty::default()
            .execute(
                "import os\n(os.getenv('PWD'), os.getenv('UNDECLARED_SECRET', 'missing'))",
                &mut filesystem,
            )
            .expect("synthetic environment should execute");
        assert_eq!(
            outcome.value,
            MontyObject::Tuple(vec![
                MontyObject::String("/workspace".to_owned()),
                MontyObject::String("missing".to_owned()),
            ])
        );
    }

    #[test]
    fn caught_secret_read_never_reaches_vfs_and_forces_final_deny() {
        let (_directory, mut filesystem) = filesystem(&[(".env", b"TOKEN=host-secret\n")]);
        let outcome = InProcessMonty::default()
            .execute(
                r"
from pathlib import Path
try:
    Path('/workspace/.env').read_text()
except PermissionError:
    Path('/workspace/safe.txt').write_text('continued')
'done'
",
                &mut filesystem,
            )
            .expect("sandboxed code may catch the policy exception");

        assert_eq!(outcome.value, MontyObject::String("done".to_owned()));
        assert_eq!(outcome.stats.read_bytes, 0);
        assert_eq!(outcome.stats.denied_accesses, 1);
        assert_eq!(outcome.denied_accesses[0].path.as_str(), ".env");
        assert!(
            !filesystem
                .read_set()
                .contains_key(&VPath::parse(".env").unwrap())
        );

        let diff = filesystem.canonical_diff().unwrap();
        let decision = TransactionPolicy::default().evaluate(PolicyInput {
            diff: &diff,
            effects: filesystem.effects(),
            denied_accesses: &outcome.denied_accesses,
            base_node_count: 2,
        });
        assert!(matches!(
            decision,
            PolicyDecision::Deny(manifest)
                if matches!(manifest.reason, DenyReason::ProtectedAccessAttempt(_))
        ));
    }

    #[test]
    fn protected_directory_contents_are_hidden_from_listing_and_direct_reads() {
        let directory = TestDirectory::new();
        let store = BlobStore::open(directory.path()).expect("blob store should open");
        let mut builder = SnapshotBuilder::new(store);
        builder
            .add_directory(VPath::parse(".env").unwrap(), 0o755)
            .unwrap();
        builder
            .add_file(VPath::parse(".env/token").unwrap(), b"host-secret", 0o600)
            .unwrap();
        builder
            .add_file(VPath::parse("safe.txt").unwrap(), b"safe", 0o644)
            .unwrap();
        let mut filesystem = VirtualFs::new(builder.build().unwrap());

        let outcome = InProcessMonty::default()
            .execute(
                r"
from pathlib import Path
visible = list(Path('/workspace').iterdir())
try:
    Path('/workspace/.env/token').read_text()
except PermissionError:
    pass
visible
",
                &mut filesystem,
            )
            .expect("protected direct access may be caught without revealing the listing");

        assert_eq!(
            outcome.value,
            MontyObject::List(vec![MontyObject::Path("/workspace/safe.txt".to_owned())])
        );
        assert_eq!(outcome.stats.read_bytes, 0);
        assert_eq!(outcome.denied_accesses.len(), 1);
        assert_eq!(outcome.denied_accesses[0].path.as_str(), ".env/token");
    }

    #[test]
    fn recursive_mkdir_authorizes_every_parent_before_virtual_mutation() {
        let (_directory, mut filesystem) = filesystem(&[]);
        let policy = CallPolicy::new(vec![
            vsh_policy::ProtectedRule::new("blocked", vsh_policy::AccessSet::ALL).unwrap(),
        ]);
        let config = InProcessConfig::default().with_call_policy(policy);
        let outcome = InProcessMonty::new(config)
            .execute(
                r"
from pathlib import Path
try:
    Path('/workspace/blocked/child').mkdir(parents=True)
except PermissionError:
    pass
'contained'
",
                &mut filesystem,
            )
            .expect("the protected parent denial may be caught by sandboxed code");

        assert_eq!(outcome.value, MontyObject::String("contained".to_owned()));
        assert_eq!(outcome.denied_accesses[0].path.as_str(), "blocked");
        assert!(filesystem.canonical_diff().unwrap().is_empty());
    }

    #[test]
    fn caught_vsh_tool_denial_is_retained_for_the_final_policy() {
        let (_directory, mut filesystem) = filesystem(&[]);
        let policy = CallPolicy::new(vec![
            vsh_policy::ProtectedRule::new("blocked", vsh_policy::AccessSet::ALL).unwrap(),
        ]);
        let config = InProcessConfig::default().with_call_policy(policy);
        let outcome = InProcessMonty::new(config)
            .execute(
                r"
try:
    vsh_write('/workspace/blocked', 'secret')
except PermissionError:
    pass
'contained'
",
                &mut filesystem,
            )
            .expect("sandboxed code may catch a VSH tool policy exception");

        assert_eq!(outcome.value, MontyObject::String("contained".to_owned()));
        assert_eq!(outcome.stats.denied_accesses, 1);
        assert_eq!(outcome.denied_accesses[0].path.as_str(), "blocked");
        assert!(filesystem.canonical_diff().unwrap().is_empty());

        let diff = filesystem.canonical_diff().unwrap();
        let decision = TransactionPolicy::default().evaluate(PolicyInput {
            diff: &diff,
            effects: filesystem.effects(),
            denied_accesses: &outcome.denied_accesses,
            base_node_count: 1,
        });
        assert!(matches!(
            decision,
            PolicyDecision::Deny(manifest)
                if matches!(manifest.reason, DenyReason::ProtectedAccessAttempt(_))
        ));
    }

    #[test]
    fn direct_vsh_search_retains_a_content_only_policy_denial() {
        let (_directory, mut filesystem) = filesystem(&[("secret.txt", b"needle")]);
        let policy = CallPolicy::new(vec![
            vsh_policy::ProtectedRule::new("secret.txt", vsh_policy::AccessSet::CONTENT_READ)
                .unwrap(),
        ]);
        let config = InProcessConfig::default().with_call_policy(policy);
        let outcome = InProcessMonty::new(config)
            .execute(
                r"
try:
    vsh_search('needle', path='/workspace/secret.txt')
except PermissionError:
    pass
'contained'
",
                &mut filesystem,
            )
            .expect("sandboxed code may catch a direct search policy exception");

        assert_eq!(outcome.value, MontyObject::String("contained".to_owned()));
        assert_eq!(outcome.stats.denied_accesses, 1);
        assert_eq!(outcome.stats.read_bytes, 0);
        assert_eq!(outcome.denied_accesses[0].path.as_str(), "secret.txt");
    }

    #[test]
    fn recursive_vsh_remove_preflights_the_entire_tree_before_mutation() {
        let directory = TestDirectory::new();
        let store = BlobStore::open(directory.path()).expect("blob store should open");
        let mut builder = SnapshotBuilder::new(store);
        builder
            .add_directory(VPath::parse("tree").unwrap(), 0o755)
            .unwrap();
        builder
            .add_directory(VPath::parse("tree/blocked").unwrap(), 0o755)
            .unwrap();
        builder
            .add_file(
                VPath::parse("tree/blocked/secret.txt").unwrap(),
                b"secret",
                0o600,
            )
            .unwrap();
        builder
            .add_file(VPath::parse("tree/safe.txt").unwrap(), b"safe", 0o644)
            .unwrap();
        let mut filesystem = VirtualFs::new(builder.build().unwrap());
        let policy = CallPolicy::new(vec![
            vsh_policy::ProtectedRule::new("tree/blocked", vsh_policy::AccessSet::ALL).unwrap(),
        ]);
        let config = InProcessConfig::default().with_call_policy(policy);
        let outcome = InProcessMonty::new(config)
            .execute(
                r"
try:
    vsh_remove('/workspace/tree', recursive=True)
except PermissionError:
    pass
'contained'
",
                &mut filesystem,
            )
            .expect("sandboxed code may catch a recursive-delete policy exception");

        assert_eq!(outcome.value, MontyObject::String("contained".to_owned()));
        assert_eq!(outcome.stats.denied_accesses, 1);
        assert_eq!(outcome.denied_accesses[0].path.as_str(), "tree/blocked");
        assert!(filesystem.canonical_diff().unwrap().is_empty());
        assert!(
            filesystem
                .exists(&VPath::parse("tree/safe.txt").unwrap())
                .unwrap()
        );
        assert!(
            filesystem
                .exists(&VPath::parse("tree/blocked/secret.txt").unwrap())
                .unwrap()
        );
    }

    #[test]
    fn python_result_validation_rejects_unprojectable_types_and_depth() {
        let nested_type = MontyObject::List(vec![MontyObject::Type(MontyType::Function)]);
        assert_eq!(
            validate_result_compatibility(&nested_type, ResultCompatibility::Python),
            Err(ResultCompatibilityError::TypeObject {
                name: "function".to_owned(),
            })
        );
        assert!(validate_result_compatibility(&nested_type, ResultCompatibility::Native).is_ok());
        assert!(
            validate_result_compatibility(
                &MontyObject::Type(MontyType::Int),
                ResultCompatibility::Python,
            )
            .is_ok()
        );

        let mut too_deep = MontyObject::None;
        for _ in 0..200 {
            too_deep = MontyObject::List(vec![too_deep]);
        }
        assert_eq!(
            validate_result_compatibility(&too_deep, ResultCompatibility::Python),
            Err(ResultCompatibilityError::Depth {
                limit: 200,
                attempted: 201,
            })
        );
    }

    #[test]
    fn mkdir_parents_and_open_append_use_only_virtual_state() {
        let (_directory, mut filesystem) = filesystem(&[]);
        let outcome = InProcessMonty::default()
            .execute(
                r"
from pathlib import Path
Path('/workspace/a/b').mkdir(parents=True)
with open('/workspace/a/b/value.txt', 'a') as handle:
    handle.write('one')
with open('/workspace/a/b/value.txt', 'a') as handle:
    handle.write('two')
Path('/workspace/a/b/value.txt').read_text()
",
                &mut filesystem,
            )
            .expect("open/append flow should execute");
        assert_eq!(outcome.value, MontyObject::String("onetwo".to_owned()));
        assert_eq!(outcome.stats.write_bytes, 6);
    }

    #[test]
    fn public_path_and_limit_errors_keep_distinct_bounded_diagnostics() {
        let root_errors = [
            VirtualRootError::NotAbsolute,
            VirtualRootError::ParentComponent,
            VirtualRootError::NulByte,
            VirtualRootError::PlatformSeparator,
            VirtualRootError::PlatformPrefix,
        ];
        assert_eq!(
            root_errors
                .map(|error| error.to_string())
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            root_errors.len()
        );

        let path_errors = [
            VirtualPathError::Empty,
            VirtualPathError::NulByte,
            VirtualPathError::EscapesAbsoluteRoot,
            VirtualPathError::OutsideRoot,
            VirtualPathError::InvalidRelative(VPath::parse("").unwrap_err()),
        ];
        for error in path_errors {
            assert!(!error.to_string().is_empty());
            assert_eq!(
                Error::source(&error).is_some(),
                matches!(error, VirtualPathError::InvalidRelative(_))
            );
        }

        let limits = [
            ExecutionLimitExceeded::ProgramBytes {
                limit: 1,
                attempted: 2,
            },
            ExecutionLimitExceeded::OsCalls {
                limit: 1,
                attempted: 2,
            },
            ExecutionLimitExceeded::ReadBytes {
                limit: 1,
                attempted: 2,
            },
            ExecutionLimitExceeded::WriteBytes {
                limit: 1,
                attempted: 2,
            },
            ExecutionLimitExceeded::ReadCallBytes {
                limit: 1,
                attempted: 2,
            },
            ExecutionLimitExceeded::WriteCallBytes {
                limit: 1,
                attempted: 2,
            },
            ExecutionLimitExceeded::PathBytes {
                limit: 1,
                attempted: 2,
            },
            ExecutionLimitExceeded::DirectoryEntries {
                limit: 1,
                attempted: 2,
            },
            ExecutionLimitExceeded::OutputBytes {
                limit: 1,
                attempted: 2,
            },
            ExecutionLimitExceeded::ResultBytes {
                limit: 1,
                attempted: 2,
            },
            ExecutionLimitExceeded::ExceptionBytes {
                limit: 1,
                attempted: 2,
            },
        ];
        assert_eq!(
            limits
                .map(|error| error.to_string())
                .into_iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len(),
            limits.len()
        );
    }

    #[test]
    fn public_result_and_execution_errors_keep_distinct_diagnostics() {
        let compatibility_errors = [
            ResultCompatibilityError::Depth {
                limit: 1,
                attempted: 2,
            },
            ResultCompatibilityError::TypeObject {
                name: "unsupported".to_owned(),
            },
        ];
        assert_ne!(
            compatibility_errors[0].to_string(),
            compatibility_errors[1].to_string()
        );

        let execution_errors = [
            ExecutionError::Limit(Box::new(ExecutionLimitExceeded::OsCalls {
                limit: 1,
                attempted: 2,
            })),
            ExecutionError::InternalVfs(Box::new(VfsError::RootMutation)),
            ExecutionError::UnsupportedSuspension {
                kind: "call",
                name: Some("name".to_owned()),
            },
            ExecutionError::UnsupportedSuspension {
                kind: "call",
                name: None,
            },
            ExecutionError::Worker(Box::new(WorkerFailure {
                kind: WorkerFailureKind::Protocol,
                detail: "bounded".to_owned(),
            })),
        ];
        for error in execution_errors {
            assert!(!error.to_string().is_empty());
        }
    }
}
