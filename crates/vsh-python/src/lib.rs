//! Thin `PyO3` bindings over the native VSH SDK.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use monty_proto::python::{InstanceStore, monty_to_py};
use pyo3::create_exception;
use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;

create_exception!(
    _native,
    VshRuntimeError,
    PyRuntimeError,
    "Base exception for typed native VSH failures."
);
create_exception!(
    _native,
    VshExecutionError,
    VshRuntimeError,
    "Monty compilation, execution, or resource-limit failure."
);
create_exception!(
    _native,
    VshStateError,
    VshRuntimeError,
    "Transaction lifecycle, approval, reservation, or replay failure."
);
create_exception!(
    _native,
    VshStaleError,
    VshRuntimeError,
    "Host dependencies changed after virtual execution."
);
create_exception!(
    _native,
    VshRecoveryError,
    VshRuntimeError,
    "Durable recovery is required or could not prove ownership."
);
create_exception!(
    _native,
    VshInternalError,
    VshRuntimeError,
    "A contained internal panic or invariant failure."
);

/// Return the canonical VSH version from the Rust facade.
#[pyfunction]
fn version() -> &'static str {
    vsh::VERSION
}

/// Return the implementation identity shared by all SDK surfaces.
#[pyfunction]
fn engine_kind() -> &'static str {
    vsh::engine_kind()
}

/// Normalize a workspace-relative path using the Rust security contract.
#[pyfunction]
fn normalize_path(path: &str) -> PyResult<String> {
    vsh::VPath::parse(path)
        .map(|value| value.as_str().to_owned())
        .map_err(|error| PyValueError::new_err(error.to_string()))
}

/// Python spelling of native preview/auto behavior.
#[pyclass(
    name = "RunMode",
    eq,
    eq_int,
    from_py_object,
    rename_all = "SCREAMING_SNAKE_CASE"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PyRunMode {
    Preview,
    Auto,
}

impl From<PyRunMode> for vsh::RunMode {
    fn from(value: PyRunMode) -> Self {
        match value {
            PyRunMode::Preview => Self::Preview,
            PyRunMode::Auto => Self::Auto,
        }
    }
}

/// Python spelling of compact/full receipt behavior.
#[pyclass(
    name = "ReceiptDetail",
    eq,
    eq_int,
    from_py_object,
    rename_all = "SCREAMING_SNAKE_CASE"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PyReceiptDetail {
    Compact,
    Full,
}

impl From<PyReceiptDetail> for vsh::ReceiptDetail {
    fn from(value: PyReceiptDetail) -> Self {
        match value {
            PyReceiptDetail::Compact => Self::Compact,
            PyReceiptDetail::Full => Self::Full,
        }
    }
}

/// Request-scoped hard resource caps.
#[pyclass(name = "ExecutionBudget", frozen, get_all, from_py_object)]
#[derive(Clone, Debug)]
#[allow(clippy::struct_field_names)]
struct PyExecutionBudget {
    max_program_bytes: usize,
    max_duration_ms: u64,
    max_recursion_depth: usize,
    max_memory_bytes: usize,
    max_os_calls: u64,
    max_read_bytes: u64,
    max_write_bytes: u64,
    max_io_call_bytes: usize,
    max_path_bytes: usize,
    max_directory_entries: u64,
    max_output_bytes: usize,
    max_result_bytes: usize,
    max_exception_bytes: usize,
}

#[pymethods]
impl PyExecutionBudget {
    #[new]
    #[pyo3(signature = (*, max_program_bytes=None, max_duration_ms=None, max_recursion_depth=None, max_memory_bytes=None, max_os_calls=None, max_read_bytes=None, max_write_bytes=None, max_io_call_bytes=None, max_path_bytes=None, max_directory_entries=None, max_output_bytes=None, max_result_bytes=None, max_exception_bytes=None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        max_program_bytes: Option<usize>,
        max_duration_ms: Option<u64>,
        max_recursion_depth: Option<usize>,
        max_memory_bytes: Option<usize>,
        max_os_calls: Option<u64>,
        max_read_bytes: Option<u64>,
        max_write_bytes: Option<u64>,
        max_io_call_bytes: Option<usize>,
        max_path_bytes: Option<usize>,
        max_directory_entries: Option<u64>,
        max_output_bytes: Option<usize>,
        max_result_bytes: Option<usize>,
        max_exception_bytes: Option<usize>,
    ) -> Self {
        let defaults = vsh::ExecutionBudget::default();
        Self {
            max_program_bytes: max_program_bytes.unwrap_or(defaults.max_program_bytes),
            max_duration_ms: max_duration_ms.unwrap_or_else(|| {
                u64::try_from(defaults.max_duration.as_millis()).unwrap_or(u64::MAX)
            }),
            max_recursion_depth: max_recursion_depth.unwrap_or(defaults.max_recursion_depth),
            max_memory_bytes: max_memory_bytes.unwrap_or(defaults.max_memory_bytes),
            max_os_calls: max_os_calls.unwrap_or(defaults.max_os_calls),
            max_read_bytes: max_read_bytes.unwrap_or(defaults.max_read_bytes),
            max_write_bytes: max_write_bytes.unwrap_or(defaults.max_write_bytes),
            max_io_call_bytes: max_io_call_bytes.unwrap_or(defaults.max_io_call_bytes),
            max_path_bytes: max_path_bytes.unwrap_or(defaults.max_path_bytes),
            max_directory_entries: max_directory_entries.unwrap_or(defaults.max_directory_entries),
            max_output_bytes: max_output_bytes.unwrap_or(defaults.max_output_bytes),
            max_result_bytes: max_result_bytes.unwrap_or(defaults.max_result_bytes),
            max_exception_bytes: max_exception_bytes.unwrap_or(defaults.max_exception_bytes),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "ExecutionBudget(max_duration_ms={}, max_memory_bytes={}, max_os_calls={}, max_read_bytes={}, max_write_bytes={})",
            self.max_duration_ms,
            self.max_memory_bytes,
            self.max_os_calls,
            self.max_read_bytes,
            self.max_write_bytes
        )
    }
}

impl Default for PyExecutionBudget {
    fn default() -> Self {
        Self::new(
            None, None, None, None, None, None, None, None, None, None, None, None, None,
        )
    }
}

impl From<PyExecutionBudget> for vsh::ExecutionBudget {
    fn from(value: PyExecutionBudget) -> Self {
        Self {
            max_program_bytes: value.max_program_bytes,
            max_duration: Duration::from_millis(value.max_duration_ms),
            max_recursion_depth: value.max_recursion_depth,
            max_memory_bytes: value.max_memory_bytes,
            max_os_calls: value.max_os_calls,
            max_read_bytes: value.max_read_bytes,
            max_write_bytes: value.max_write_bytes,
            max_io_call_bytes: value.max_io_call_bytes,
            max_path_bytes: value.max_path_bytes,
            max_directory_entries: value.max_directory_entries,
            max_output_bytes: value.max_output_bytes,
            max_result_bytes: value.max_result_bytes,
            max_exception_bytes: value.max_exception_bytes,
        }
    }
}

/// One owned Python request converted exactly once before the GIL is released.
#[pyclass(name = "RunRequest", frozen, get_all, skip_from_py_object)]
#[derive(Clone, Debug)]
struct PyRunRequest {
    code: String,
    intent: Option<String>,
    mode: PyRunMode,
    detail: PyReceiptDetail,
    budget: PyExecutionBudget,
}

#[pymethods]
impl PyRunRequest {
    #[new]
    #[pyo3(signature = (code, *, intent=None, mode=None, detail=None, budget=None))]
    fn new(
        code: String,
        intent: Option<String>,
        mode: Option<PyRunMode>,
        detail: Option<PyReceiptDetail>,
        budget: Option<PyExecutionBudget>,
    ) -> Self {
        Self {
            code,
            intent,
            mode: mode.unwrap_or(PyRunMode::Preview),
            detail: detail.unwrap_or(PyReceiptDetail::Compact),
            budget: budget.unwrap_or_default(),
        }
    }

    fn __repr__(&self) -> String {
        format!(
            "RunRequest(code_bytes={}, mode={:?}, detail={:?})",
            self.code.len(),
            self.mode,
            self.detail
        )
    }
}

#[derive(Clone)]
struct OwnedRunRequest {
    code: String,
    intent: Option<String>,
    mode: vsh::RunMode,
    detail: vsh::ReceiptDetail,
    budget: vsh::ExecutionBudget,
}

impl From<&PyRunRequest> for OwnedRunRequest {
    fn from(value: &PyRunRequest) -> Self {
        Self {
            code: value.code.clone(),
            intent: value.intent.clone(),
            mode: value.mode.into(),
            detail: value.detail.into(),
            budget: value.budget.clone().into(),
        }
    }
}

impl OwnedRunRequest {
    fn borrowed(&self) -> vsh::RunRequest<'_> {
        vsh::RunRequest {
            code: &self.code,
            intent: self.intent.as_deref(),
            mode: self.mode,
            detail: self.detail,
            budget: self.budget,
        }
    }

    fn preview_input(
        request: &Bound<'_, PyAny>,
        intent: Option<String>,
        detail: Option<PyReceiptDetail>,
        budget: Option<PyExecutionBudget>,
    ) -> PyResult<Self> {
        if request.is_instance_of::<PyRunRequest>() {
            let request = request.extract::<PyRef<'_, PyRunRequest>>()?;
            if intent.is_some() || detail.is_some() || budget.is_some() {
                return Err(PyTypeError::new_err(
                    "intent, detail, and budget are only valid when preview() receives source code",
                ));
            }
            return Ok(Self::from(&*request));
        }

        let code = request.extract::<String>().map_err(|_| {
            PyTypeError::new_err("preview() requires a RunRequest or source-code str")
        })?;
        Ok(Self {
            code,
            intent,
            mode: vsh::RunMode::Preview,
            detail: detail.unwrap_or(PyReceiptDetail::Compact).into(),
            budget: budget.unwrap_or_default().into(),
        })
    }
}

/// Python projection of one native VSH receipt.
#[pyclass(name = "Receipt", frozen, get_all, skip_from_py_object)]
#[derive(Debug)]
struct PyReceipt {
    transaction: String,
    base_snapshot: String,
    state: String,
    decision: String,
    diff: String,
    changed_paths: usize,
    changes: Vec<(String, String)>,
    result: Py<PyAny>,
    stdout: String,
    risk_flags: Vec<String>,
    deny_reason: Option<String>,
    os_calls: u64,
    read_bytes: u64,
    write_bytes: u64,
    directory_entries: u64,
    output_bytes: usize,
    denied_accesses: u64,
    result_bytes: u64,
    committed: bool,
    commit_operations: Option<usize>,
    verified_paths: Option<usize>,
    cleanup_pending: bool,
    snapshot_ns: u64,
    execute_ns: u64,
    diff_ns: u64,
    policy_ns: u64,
    bind_and_store_ns: u64,
    commit_ns: u64,
    total_ns: u64,
}

#[pymethods]
impl PyReceipt {
    fn __repr__(&self) -> String {
        format!(
            "Receipt(transaction='{}', state='{}', decision='{}', changed_paths={})",
            self.transaction, self.state, self.decision, self.changed_paths
        )
    }

    /// Return stage timings without storing a Python dictionary in the core path.
    fn timings_ns(&self) -> BTreeMap<&'static str, u64> {
        BTreeMap::from([
            ("snapshot", self.snapshot_ns),
            ("execute", self.execute_ns),
            ("diff", self.diff_ns),
            ("policy", self.policy_ns),
            ("bind_and_store", self.bind_and_store_ns),
            ("commit", self.commit_ns),
            ("total", self.total_ns),
        ])
    }

    /// Compute Python's actual representation only when the caller requests it.
    #[getter]
    fn result_repr(&self, py: Python<'_>) -> PyResult<String> {
        self.result.bind(py).repr()?.extract()
    }
}

impl PyReceipt {
    fn from_native(py: Python<'_>, receipt: vsh::Receipt) -> PyResult<Self> {
        let (decision, risk_flags, deny_reason) = decision_projection(&receipt.decision);
        let commit_operations = receipt.commit.as_ref().map(|commit| commit.operations);
        let verified_paths = receipt.commit.as_ref().map(|commit| commit.verified_paths);
        let cleanup_pending = receipt
            .commit
            .as_ref()
            .is_some_and(|commit| commit.cleanup_pending);
        let result = monty_to_py(py, &receipt.value, &InstanceStore::new(py))?;
        Ok(Self {
            transaction: receipt.transaction.to_string(),
            base_snapshot: receipt.base_snapshot.to_string(),
            state: state_name(receipt.state).to_owned(),
            decision: decision.to_owned(),
            diff: receipt.diff.to_string(),
            changed_paths: receipt.changed_paths,
            changes: receipt
                .changes
                .iter()
                .map(|entry| {
                    (
                        entry.path.as_str().to_owned(),
                        diff_kind_name(entry.kind).to_owned(),
                    )
                })
                .collect(),
            result,
            stdout: receipt.stdout,
            risk_flags,
            deny_reason,
            os_calls: receipt.execution.os_calls,
            read_bytes: receipt.execution.read_bytes,
            write_bytes: receipt.execution.write_bytes,
            directory_entries: receipt.execution.directory_entries,
            output_bytes: receipt.execution.output_bytes,
            denied_accesses: receipt.execution.denied_accesses,
            result_bytes: receipt.execution.result_bytes,
            committed: receipt.state == vsh::TransactionState::Committed,
            commit_operations,
            verified_paths,
            cleanup_pending,
            snapshot_ns: receipt.timings.snapshot_ns,
            execute_ns: receipt.timings.execute_ns,
            diff_ns: receipt.timings.diff_ns,
            policy_ns: receipt.timings.policy_ns,
            bind_and_store_ns: receipt.timings.bind_and_store_ns,
            commit_ns: receipt.timings.commit_ns,
            total_ns: receipt.timings.total_ns,
        })
    }
}

/// Python projection of startup/manual crash recovery.
#[pyclass(name = "RecoveryReport", frozen, get_all, skip_from_py_object)]
#[derive(Clone, Debug)]
struct PyRecoveryReport {
    finalized_commits: usize,
    rolled_back: usize,
    cleaned: usize,
    orphaned: usize,
    conflicts: Vec<(String, Option<String>, String)>,
}

impl From<vsh::RecoveryReport> for PyRecoveryReport {
    fn from(report: vsh::RecoveryReport) -> Self {
        Self {
            finalized_commits: report.finalized_commits,
            rolled_back: report.rolled_back,
            cleaned: report.cleaned,
            orphaned: report.orphaned,
            conflicts: report
                .conflicts
                .into_iter()
                .map(|conflict| {
                    (
                        conflict.transaction.to_string(),
                        conflict.path.map(|path| path.as_str().to_owned()),
                        conflict.reason.to_owned(),
                    )
                })
                .collect(),
        }
    }
}

/// Python owner of one native runtime. Independent instances share no execution lock.
#[pyclass(name = "Runtime", frozen)]
struct PyRuntime {
    inner: Arc<vsh::Runtime>,
}

#[pymethods]
impl PyRuntime {
    /// Open a capability-rooted workspace and recover interrupted commits.
    #[staticmethod]
    #[pyo3(signature = (workspace, *, data_directory=None, policy="balanced", worker_path=None))]
    fn open(
        py: Python<'_>,
        workspace: PathBuf,
        data_directory: Option<PathBuf>,
        policy: &str,
        worker_path: Option<PathBuf>,
    ) -> PyResult<Self> {
        let profile = match policy {
            "balanced" => vsh::PolicyProfile::Balanced,
            "strict" => vsh::PolicyProfile::Strict,
            "paranoid" => vsh::PolicyProfile::Paranoid,
            value => {
                return Err(PyValueError::new_err(format!(
                    "unknown policy profile {value:?}; expected balanced, strict, or paranoid"
                )));
            }
        };
        let mut config = vsh::RuntimeConfig::new(workspace)
            .with_policy_profile(profile)
            .with_result_compatibility(vsh::ResultCompatibility::Python);
        if let Some(data_directory) = data_directory {
            config = config.with_data_directory(data_directory);
        }
        if let Some(worker_path) = worker_path {
            config = config.with_worker_path(worker_path);
        } else if std::env::var_os("VSH_MONTY_WORKER").is_none()
            && let Some(worker_path) = python_scripts_worker(py)
        {
            config = config.with_worker_path(worker_path);
        }
        detach_open(py, config).map(|inner| Self { inner })
    }

    /// Execute one complete native transaction while the Python GIL is released.
    fn run(&self, py: Python<'_>, request: &PyRunRequest) -> PyResult<PyReceipt> {
        let request = OwnedRunRequest::from(request);
        let runtime = Arc::clone(&self.inner);
        let receipt = detach_call(py, runtime, move |runtime| {
            runtime.run(request.borrowed()).map_err(Box::new)
        })?;
        PyReceipt::from_native(py, receipt)
    }

    /// Execute policy-bound virtual state from a request or source code.
    #[pyo3(signature = (request, *, intent=None, detail=None, budget=None))]
    fn preview(
        &self,
        py: Python<'_>,
        request: &Bound<'_, PyAny>,
        intent: Option<String>,
        detail: Option<PyReceiptDetail>,
        budget: Option<PyExecutionBudget>,
    ) -> PyResult<PyReceipt> {
        let request = OwnedRunRequest::preview_input(request, intent, detail, budget)?;
        let runtime = Arc::clone(&self.inner);
        let receipt = detach_call(py, runtime, move |runtime| {
            runtime.preview(request.borrowed()).map_err(Box::new)
        })?;
        PyReceipt::from_native(py, receipt)
    }

    /// Forget a process-local auto-approved preview without mutating the host.
    fn discard_preview(&self, py: Python<'_>, transaction: &str) -> PyResult<bool> {
        let transaction = parse_transaction(transaction)?;
        let runtime = Arc::clone(&self.inner);
        detach_call(py, runtime, move |runtime| {
            runtime.discard_preview(transaction).map_err(Box::new)
        })
    }

    /// Bind an independent principal's expiring approval to an exact transaction.
    fn approve(
        &self,
        py: Python<'_>,
        transaction: &str,
        principal: &str,
        issued_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> PyResult<String> {
        let transaction = parse_transaction(transaction)?;
        let principal = vsh::PrincipalId::digest_label(principal);
        let runtime = Arc::clone(&self.inner);
        detach_call(py, runtime, move |runtime| {
            runtime
                .approve(
                    transaction,
                    principal,
                    issued_at_unix_ms,
                    expires_at_unix_ms,
                )
                .map_err(Box::new)
        })
        .map(|record| state_name(record.state()).to_owned())
    }

    /// Consume a single-use reservation and commit a previewed transaction.
    fn commit(&self, py: Python<'_>, transaction: &str, now_unix_ms: u64) -> PyResult<PyReceipt> {
        let transaction = parse_transaction(transaction)?;
        let runtime = Arc::clone(&self.inner);
        let receipt = detach_call(py, runtime, move |runtime| {
            runtime.commit(transaction, now_unix_ms).map_err(Box::new)
        })?;
        PyReceipt::from_native(py, receipt)
    }

    /// Recover durable interrupted commits while the GIL is released.
    fn recover(&self, py: Python<'_>) -> PyResult<PyRecoveryReport> {
        let runtime = Arc::clone(&self.inner);
        detach_call(py, runtime, |runtime| runtime.recover().map_err(Box::new)).map(Into::into)
    }

    fn __repr__(&self) -> String {
        let recovery = self.inner.startup_recovery();
        format!(
            "Runtime(engine='rust', startup_recovered={})",
            recovery.finalized_commits + recovery.rolled_back
        )
    }
}

fn python_scripts_worker(py: Python<'_>) -> Option<PathBuf> {
    let scripts = py
        .import("sysconfig")
        .ok()?
        .call_method1("get_path", ("scripts",))
        .ok()?
        .extract::<PathBuf>()
        .ok()?;
    let filename = if cfg!(windows) {
        "vsh-monty-worker.exe"
    } else {
        "vsh-monty-worker"
    };
    let worker = scripts.join(filename);
    worker.is_file().then_some(worker)
}

enum DetachedFailure {
    Core(Box<vsh::VshError>),
    Recovery(Box<vsh::VshError>),
    Panic,
}

fn detach_open(py: Python<'_>, config: vsh::RuntimeConfig) -> PyResult<Arc<vsh::Runtime>> {
    let result = py.detach(move || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            vsh::Runtime::open(config).map_err(Box::new)
        }))
        .map_err(|_| DetachedFailure::Panic)?
        .map(Arc::new)
        .map_err(DetachedFailure::Core)
    });
    result.map_err(map_detached_failure)
}

fn detach_call<T, F>(py: Python<'_>, runtime: Arc<vsh::Runtime>, operation: F) -> PyResult<T>
where
    T: Send,
    F: FnOnce(&vsh::Runtime) -> Result<T, Box<vsh::VshError>> + Send,
{
    let result = py.detach(move || {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(&runtime))) {
            Ok(result) => match result {
                Ok(value) => Ok(value),
                Err(error) if requires_recovery(&error) => match runtime.recover() {
                    Ok(_) => Err(DetachedFailure::Core(error)),
                    Err(recovery) => Err(DetachedFailure::Recovery(Box::new(recovery))),
                },
                Err(error) => Err(DetachedFailure::Core(error)),
            },
            Err(_) => match runtime.recover() {
                Ok(_) => Err(DetachedFailure::Panic),
                Err(recovery) => Err(DetachedFailure::Recovery(Box::new(recovery))),
            },
        }
    });
    result.map_err(map_detached_failure)
}

fn requires_recovery(error: &vsh::VshError) -> bool {
    matches!(
        error,
        vsh::VshError::Commit(
            vsh::CommitError::RecoveryRequired { .. } | vsh::CommitError::RecoveryConflict(_)
        )
    )
}

fn map_detached_failure(error: DetachedFailure) -> PyErr {
    match error {
        DetachedFailure::Core(source) => map_vsh_error(*source),
        DetachedFailure::Recovery(source) => VshRecoveryError::new_err(format!(
            "automatic recovery after a failed native call also failed: {source}"
        )),
        DetachedFailure::Panic => VshInternalError::new_err(
            "native VSH panicked; the unwind was contained at the PyO3 boundary",
        ),
    }
}

fn map_vsh_error(error: vsh::VshError) -> PyErr {
    match error {
        vsh::VshError::UnsafeDataDirectory { .. } => PyValueError::new_err(error.to_string()),
        vsh::VshError::Execution(source) => VshExecutionError::new_err(source.to_string()),
        vsh::VshError::ResultCompatibility(source) => {
            VshExecutionError::new_err(source.to_string())
        }
        vsh::VshError::Store(source) | vsh::VshError::Commit(vsh::CommitError::Store(source)) => {
            VshStateError::new_err(source.to_string())
        }
        vsh::VshError::Commit(vsh::CommitError::Stale { conflicts }) => {
            VshStaleError::new_err(format!(
                "transaction dependencies are stale ({} conflict(s))",
                conflicts.len()
            ))
        }
        vsh::VshError::Commit(
            source @ (vsh::CommitError::RecoveryRequired { .. }
            | vsh::CommitError::RecoveryConflict(_)),
        ) => VshRecoveryError::new_err(source.to_string()),
        vsh::VshError::RecoveryConflicts(report) => VshRecoveryError::new_err(format!(
            "startup recovery left {} ambiguous transaction(s)",
            report.conflicts.len()
        )),
        source @ (vsh::VshError::MissingPending { .. }
        | vsh::VshError::DuplicatePending { .. }
        | vsh::VshError::EphemeralCapacity { .. }
        | vsh::VshError::Approval(_)) => VshStateError::new_err(source.to_string()),
        source => VshRuntimeError::new_err(source.to_string()),
    }
}

fn parse_transaction(value: &str) -> PyResult<vsh::TransactionId> {
    value
        .parse()
        .map_err(|error: vsh::ParseDigestError| PyValueError::new_err(error.to_string()))
}

fn decision_projection(
    decision: &vsh::RuntimeDecision,
) -> (&'static str, Vec<String>, Option<String>) {
    match decision {
        vsh::RuntimeDecision::Denied(manifest) => (
            "denied",
            Vec::new(),
            Some(deny_reason_name(&manifest.reason).to_owned()),
        ),
        vsh::RuntimeDecision::AutoApproved => ("auto_approved", Vec::new(), None),
        vsh::RuntimeDecision::PendingApproval(manifest) => (
            "pending_approval",
            manifest
                .flags
                .iter()
                .map(|flag| risk_flag_name(*flag).to_owned())
                .collect(),
            None,
        ),
    }
}

fn state_name(state: vsh::TransactionState) -> &'static str {
    match state {
        vsh::TransactionState::Created => "created",
        vsh::TransactionState::Running => "running",
        vsh::TransactionState::VirtualComplete => "virtual_complete",
        vsh::TransactionState::Denied => "denied",
        vsh::TransactionState::AutoApproved => "auto_approved",
        vsh::TransactionState::PendingApproval => "pending_approval",
        vsh::TransactionState::Approved => "approved",
        vsh::TransactionState::Reserved => "reserved",
        vsh::TransactionState::Revalidating => "revalidating",
        vsh::TransactionState::Committing => "committing",
        vsh::TransactionState::Committed => "committed",
        vsh::TransactionState::Stale => "stale",
        vsh::TransactionState::Expired => "expired",
        vsh::TransactionState::RecoveryRequired => "recovery_required",
        vsh::TransactionState::Failed => "failed",
        _ => "unknown",
    }
}

fn diff_kind_name(kind: vsh::DiffKind) -> &'static str {
    match kind {
        vsh::DiffKind::Create => "create",
        vsh::DiffKind::Delete => "delete",
        vsh::DiffKind::Modify => "modify",
        vsh::DiffKind::MetadataChange => "metadata_change",
    }
}

fn risk_flag_name(flag: vsh::RiskFlag) -> &'static str {
    match flag {
        vsh::RiskFlag::Mutation => "mutation",
        vsh::RiskFlag::Deletion => "deletion",
        vsh::RiskFlag::Rename => "rename",
        vsh::RiskFlag::ExecutableChange => "executable_change",
        vsh::RiskFlag::SymlinkChange => "symlink_change",
        vsh::RiskFlag::LargeTouchedSet => "large_touched_set",
        vsh::RiskFlag::LargeByteChange => "large_byte_change",
    }
}

fn deny_reason_name(reason: &vsh::DenyReason) -> &'static str {
    match reason {
        vsh::DenyReason::ProtectedAccessAttempt(_) => "protected_access_attempt",
        vsh::DenyReason::ProtectedMutation(_) => "protected_mutation",
        vsh::DenyReason::TouchedPathLimit { .. } => "touched_path_limit",
        vsh::DenyReason::ChangedByteLimit { .. } => "changed_byte_limit",
        vsh::DenyReason::DeletePathLimit { .. } => "delete_path_limit",
        vsh::DenyReason::DeleteRatioLimit { .. } => "delete_ratio_limit",
        _ => "unknown",
    }
}

/// Native implementation module imported as `vsh._native`.
#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("__version__", vsh::VERSION)?;
    module.add("VshRuntimeError", module.py().get_type::<VshRuntimeError>())?;
    module.add(
        "VshExecutionError",
        module.py().get_type::<VshExecutionError>(),
    )?;
    module.add("VshStateError", module.py().get_type::<VshStateError>())?;
    module.add("VshStaleError", module.py().get_type::<VshStaleError>())?;
    module.add(
        "VshRecoveryError",
        module.py().get_type::<VshRecoveryError>(),
    )?;
    module.add(
        "VshInternalError",
        module.py().get_type::<VshInternalError>(),
    )?;
    module.add_class::<PyRunMode>()?;
    module.add_class::<PyReceiptDetail>()?;
    module.add_class::<PyExecutionBudget>()?;
    module.add_class::<PyRunRequest>()?;
    module.add_class::<PyReceipt>()?;
    module.add_class::<PyRecoveryReport>()?;
    module.add_class::<PyRuntime>()?;
    module.add_function(wrap_pyfunction!(version, module)?)?;
    module.add_function(wrap_pyfunction!(engine_kind, module)?)?;
    module.add_function(wrap_pyfunction!(normalize_path, module)?)?;
    Ok(())
}
