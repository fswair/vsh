//! Thin `PyO3` bindings over the native VSH SDK.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use monty_proto::python::{InstanceStore, monty_to_py};
use pyo3::create_exception;
use pyo3::exceptions::{PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

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

/// Python spelling of commit-hook policy scope.
#[pyclass(
    name = "HookScope",
    eq,
    eq_int,
    from_py_object,
    rename_all = "SCREAMING_SNAKE_CASE"
)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PyHookScope {
    ReviewRequired,
    AllRequests,
}

impl From<PyHookScope> for vsh::HookScope {
    fn from(value: PyHookScope) -> Self {
        match value {
            PyHookScope::ReviewRequired => Self::ReviewRequired,
            PyHookScope::AllRequests => Self::AllRequests,
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

/// Metadata/content identity for one side of a canonical change.
#[pyclass(name = "NodeSummary", frozen, get_all, skip_from_py_object)]
#[derive(Clone, Debug)]
struct PyNodeSummary {
    kind: String,
    size: u64,
    mode: u32,
    content: Option<String>,
}

impl From<vsh::NodeState> for PyNodeSummary {
    fn from(state: vsh::NodeState) -> Self {
        Self {
            kind: node_kind_name(state.kind()).to_owned(),
            size: state.size(),
            mode: state.mode(),
            content: state.content().map(|content| match content {
                vsh::ContentVersion::Blob(value) => value.to_string(),
                vsh::ContentVersion::Stamp(value) => format!(
                    "stamp:{}:{}:{}:{}:{}:{}:{}",
                    node_kind_name(value.kind),
                    value.size,
                    value.mode,
                    value.mtime_ns,
                    value
                        .ctime_ns
                        .map_or_else(|| "none".to_owned(), |item| item.to_string()),
                    value.file_id.high,
                    value.file_id.low,
                ),
                _ => "unknown".to_owned(),
            }),
        }
    }
}

/// One entry from the exact path-ordered canonical diff.
#[pyclass(name = "CanonicalChange", frozen, get_all, skip_from_py_object)]
#[derive(Clone, Debug)]
struct PyCanonicalChange {
    path: String,
    kind: String,
    before: Option<PyNodeSummary>,
    after: Option<PyNodeSummary>,
}

impl From<&vsh::DiffEntry> for PyCanonicalChange {
    fn from(entry: &vsh::DiffEntry) -> Self {
        Self {
            path: entry.path.as_str().to_owned(),
            kind: diff_kind_name(entry.kind).to_owned(),
            before: entry.before.map(Into::into),
            after: entry.after.map(Into::into),
        }
    }
}

/// Ordered operation-level evidence produced by virtual execution.
#[pyclass(name = "EffectSummary", frozen, get_all, skip_from_py_object)]
#[derive(Clone, Debug)]
struct PyEffectSummary {
    sequence: u64,
    origin: String,
    operation: String,
    paths: Vec<String>,
    before: Option<PyNodeSummary>,
    after: Option<PyNodeSummary>,
    observed_content: Option<String>,
}

impl From<&vsh::EffectEvent> for PyEffectSummary {
    fn from(event: &vsh::EffectEvent) -> Self {
        let (operation, paths, before, after, observed_content) = match &event.effect {
            vsh::Effect::MetadataRead { path, state } => (
                "metadata_read",
                vec![path.as_str().to_owned()],
                None,
                state.map(Into::into),
                None,
            ),
            vsh::Effect::ContentRead { path, blob } => (
                "content_read",
                vec![path.as_str().to_owned()],
                None,
                None,
                Some(blob.to_string()),
            ),
            vsh::Effect::DirectoryRead { path, digest } => (
                "directory_read",
                vec![path.as_str().to_owned()],
                None,
                None,
                Some(digest.to_string()),
            ),
            vsh::Effect::Create { path, after } => (
                "create",
                vec![path.as_str().to_owned()],
                None,
                Some((*after).into()),
                None,
            ),
            vsh::Effect::ModifyContent {
                path,
                before,
                after,
            } => (
                "modify_content",
                vec![path.as_str().to_owned()],
                Some((*before).into()),
                Some((*after).into()),
                None,
            ),
            vsh::Effect::Delete { path, before } => (
                "delete",
                vec![path.as_str().to_owned()],
                Some((*before).into()),
                None,
                None,
            ),
            vsh::Effect::Rename {
                from,
                to,
                before,
                after,
            } => (
                "rename",
                vec![from.as_str().to_owned(), to.as_str().to_owned()],
                Some((*before).into()),
                Some((*after).into()),
                None,
            ),
            _ => ("unknown", Vec::new(), None, None, None),
        };
        Self {
            sequence: event.sequence,
            origin: effect_origin_name(event.origin).to_owned(),
            operation: operation.to_owned(),
            paths,
            before,
            after,
            observed_content,
        }
    }
}

/// Immutable evidence delivered to a Python commit hook.
#[pyclass(name = "ReviewContent", frozen, skip_from_py_object)]
#[derive(Clone, Debug)]
struct PyReviewContent {
    #[pyo3(get)]
    path: String,
    #[pyo3(get)]
    blob: String,
    bytes: Vec<u8>,
}

#[pymethods]
impl PyReviewContent {
    #[getter]
    fn bytes<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.bytes)
    }
}

impl From<&vsh::ReviewContent> for PyReviewContent {
    fn from(content: &vsh::ReviewContent) -> Self {
        Self {
            path: content.path.as_str().to_owned(),
            blob: content.blob.to_string(),
            bytes: content.bytes.clone(),
        }
    }
}

/// Immutable evidence delivered to a Python commit hook.
#[pyclass(name = "RequestEvent", frozen, get_all, skip_from_py_object)]
#[derive(Clone, Debug)]
struct PyRequestEvent {
    schema_version: u16,
    event_id: String,
    hook_id: String,
    transaction: String,
    state: String,
    baseline: String,
    base_snapshot: String,
    diff: String,
    read_set: String,
    write_set: String,
    program: String,
    policy: String,
    runtime_config: String,
    intent_digest: Option<String>,
    intent: Option<String>,
    policy_profile: String,
    policy_thresholds: BTreeMap<String, u64>,
    risk_flags: Vec<String>,
    touched_paths: usize,
    created_paths: usize,
    modified_paths: usize,
    deleted_paths: usize,
    renamed_paths: usize,
    changed_bytes: u64,
    delete_ratio_bps: u16,
    executable_changes: usize,
    symlink_changes: usize,
    canonical_diff: Vec<PyCanonicalChange>,
    effects: Vec<PyEffectSummary>,
    os_calls: u64,
    read_bytes: u64,
    write_bytes: u64,
    directory_entries: u64,
    output_bytes: usize,
    denied_accesses: u64,
    result_bytes: u64,
    evidence_complete: bool,
    evidence_truncated: bool,
    contents: Vec<PyReviewContent>,
    content_complete: bool,
}

impl From<&vsh::RequestEvent> for PyRequestEvent {
    fn from(event: &vsh::RequestEvent) -> Self {
        let metrics = event.risk_metrics;
        let thresholds = event.policy_thresholds;
        Self {
            schema_version: event.schema_version,
            event_id: event.event_id.to_string(),
            hook_id: event.hook_id.to_string(),
            transaction: event.transaction.to_string(),
            state: state_name(event.state).to_owned(),
            baseline: match event.baseline {
                vsh::HookBaseline::AutoApproved => "auto_approved",
                vsh::HookBaseline::ReviewRequired => "review_required",
            }
            .to_owned(),
            base_snapshot: event.base_snapshot.to_string(),
            diff: event.diff.to_string(),
            read_set: event.read_set.to_string(),
            write_set: event.write_set.to_string(),
            program: event.program.to_string(),
            policy: event.policy.to_string(),
            runtime_config: event.runtime_config.to_string(),
            intent_digest: event.intent_digest.map(|value| value.to_string()),
            intent: event.intent.clone(),
            policy_profile: policy_profile_name(event.policy_profile).to_owned(),
            policy_thresholds: [
                (
                    "escalate_touched_paths",
                    u64::try_from(thresholds.escalate_touched_paths).unwrap_or(u64::MAX),
                ),
                ("escalate_changed_bytes", thresholds.escalate_changed_bytes),
                (
                    "deny_touched_paths",
                    u64::try_from(thresholds.deny_touched_paths).unwrap_or(u64::MAX),
                ),
                ("deny_changed_bytes", thresholds.deny_changed_bytes),
                (
                    "deny_deleted_paths",
                    u64::try_from(thresholds.deny_deleted_paths).unwrap_or(u64::MAX),
                ),
                (
                    "delete_ratio_minimum_paths",
                    u64::try_from(thresholds.delete_ratio_minimum_paths).unwrap_or(u64::MAX),
                ),
                (
                    "deny_delete_ratio_bps",
                    u64::from(thresholds.deny_delete_ratio_bps),
                ),
            ]
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
            risk_flags: event
                .risk_flags
                .iter()
                .map(|flag| risk_flag_name(*flag).to_owned())
                .collect(),
            touched_paths: metrics.touched_paths,
            created_paths: metrics.created_paths,
            modified_paths: metrics.modified_paths,
            deleted_paths: metrics.deleted_paths,
            renamed_paths: metrics.renamed_paths,
            changed_bytes: metrics.changed_bytes,
            delete_ratio_bps: metrics.delete_ratio_bps,
            executable_changes: metrics.executable_changes,
            symlink_changes: metrics.symlink_changes,
            canonical_diff: event.canonical_diff.iter().map(Into::into).collect(),
            effects: event.effects.iter().map(Into::into).collect(),
            os_calls: event.execution.os_calls,
            read_bytes: event.execution.read_bytes,
            write_bytes: event.execution.write_bytes,
            directory_entries: event.execution.directory_entries,
            output_bytes: event.execution.output_bytes,
            denied_accesses: event.execution.denied_accesses,
            result_bytes: event.execution.result_bytes,
            evidence_complete: event.evidence_complete,
            evidence_truncated: event.evidence_truncated,
            contents: event.contents.iter().map(Into::into).collect(),
            content_complete: event.content_complete,
        }
    }
}

/// Opaque native prepare result carried across a Python handler await point.
#[pyclass(name = "CommitPreparation", frozen, skip_from_py_object)]
#[derive(Clone, Debug)]
struct PyCommitPreparation {
    inner: vsh::CommitPreparation,
    event: Option<PyRequestEvent>,
}

#[pymethods]
impl PyCommitPreparation {
    #[getter]
    fn transaction(&self) -> String {
        self.inner.transaction().to_string()
    }

    #[getter]
    fn event(&self) -> Option<PyRequestEvent> {
        self.event.clone()
    }
}

impl From<vsh::CommitPreparation> for PyCommitPreparation {
    fn from(inner: vsh::CommitPreparation) -> Self {
        let event = inner.event().map(Into::into);
        Self { inner, event }
    }
}

/// Typed decision returned by a Python hook handler.
#[pyclass(name = "HookDecision", frozen, skip_from_py_object)]
#[derive(Clone, Debug)]
struct PyHookDecision {
    inner: vsh::HookDecision,
}

#[pymethods]
impl PyHookDecision {
    #[staticmethod]
    fn follow_policy() -> Self {
        Self {
            inner: vsh::HookDecision::FollowPolicy,
        }
    }

    #[staticmethod]
    fn approve(reason: String) -> Self {
        Self {
            inner: vsh::HookDecision::approve(reason),
        }
    }

    #[staticmethod]
    fn review(feedback: String) -> Self {
        Self {
            inner: vsh::HookDecision::review(feedback),
        }
    }

    #[staticmethod]
    fn reject(reason: String) -> Self {
        Self {
            inner: vsh::HookDecision::reject(reason),
        }
    }
}

/// Hook-decision provenance returned with commit resolution.
#[pyclass(name = "HookDecisionRecord", frozen, get_all, skip_from_py_object)]
#[derive(Clone, Debug)]
struct PyHookDecisionRecord {
    event_id: String,
    hook_id: String,
    verdict: String,
    reason: String,
    principal: Option<String>,
}

impl From<vsh::HookDecisionRecord> for PyHookDecisionRecord {
    fn from(record: vsh::HookDecisionRecord) -> Self {
        Self {
            event_id: record.event_id.to_string(),
            hook_id: record.hook_id.to_string(),
            verdict: hook_verdict_name(record.verdict).to_owned(),
            reason: record.reason,
            principal: record.principal.map(|value| value.to_string()),
        }
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

/// Receipt plus the decision provenance produced by hook resolution.
#[pyclass(name = "CommitResolution", frozen, get_all, skip_from_py_object)]
#[derive(Debug)]
struct PyCommitResolution {
    receipt: Py<PyReceipt>,
    hook: Option<PyHookDecisionRecord>,
}

impl PyCommitResolution {
    fn from_native(py: Python<'_>, resolution: vsh::CommitResolution) -> PyResult<Self> {
        Ok(Self {
            receipt: Py::new(py, PyReceipt::from_native(py, resolution.receipt)?)?,
            hook: resolution.hook.map(Into::into),
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
    #[pyo3(signature = (workspace, *, data_directory=None, policy="balanced", worker_path=None, hook_id=None, hook_scope=None, review_content_bytes=0))]
    // Preserve Python's keyword-only options without a redundant config wrapper.
    #[allow(clippy::too_many_arguments)]
    fn open(
        py: Python<'_>,
        workspace: PathBuf,
        data_directory: Option<PathBuf>,
        policy: &str,
        worker_path: Option<PathBuf>,
        hook_id: Option<&str>,
        hook_scope: Option<PyHookScope>,
        review_content_bytes: usize,
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
        if hook_id.is_some() || hook_scope.is_some() {
            config = config.with_commit_hook(
                vsh::HookConfig::new(hook_id.unwrap_or("vsh.python-hook"))
                    .with_scope(hook_scope.unwrap_or(PyHookScope::ReviewRequired).into())
                    .with_max_content_bytes(review_content_bytes),
            );
        } else if review_content_bytes > 0 {
            return Err(PyValueError::new_err(
                "review_content_bytes requires a configured commit hook",
            ));
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

    /// Freeze an exact hook event without executing caller code under native locks.
    fn prepare_commit(&self, py: Python<'_>, transaction: &str) -> PyResult<PyCommitPreparation> {
        let transaction = parse_transaction(transaction)?;
        let runtime = Arc::clone(&self.inner);
        detach_call(py, runtime, move |runtime| {
            runtime.prepare_commit(transaction).map_err(Box::new)
        })
        .map(Into::into)
    }

    /// Revalidate and apply a typed decision to one opaque preparation.
    fn resolve_commit(
        &self,
        py: Python<'_>,
        preparation: &PyCommitPreparation,
        decision: &PyHookDecision,
        now_unix_ms: u64,
    ) -> PyResult<PyCommitResolution> {
        let preparation = preparation.inner.clone();
        let decision = decision.inner.clone();
        let runtime = Arc::clone(&self.inner);
        let resolution = detach_call(py, runtime, move |runtime| {
            runtime
                .resolve_commit(&preparation, &decision, now_unix_ms)
                .map_err(Box::new)
        })?;
        PyCommitResolution::from_native(py, resolution)
    }

    /// Apply fail-closed state after a Python handler exception or cancellation.
    fn fail_hook(&self, py: Python<'_>, preparation: &PyCommitPreparation) -> PyResult<()> {
        let preparation = preparation.inner.clone();
        let runtime = Arc::clone(&self.inner);
        detach_call(py, runtime, move |runtime| {
            runtime.fail_hook(&preparation).map_err(Box::new)
        })
    }

    /// Return one durable transaction state without loading result data into Python.
    fn transaction_state(&self, py: Python<'_>, transaction: &str) -> PyResult<String> {
        let transaction = parse_transaction(transaction)?;
        let runtime = Arc::clone(&self.inner);
        detach_call(py, runtime, move |runtime| {
            runtime.transaction(transaction).map_err(Box::new)
        })
        .map(|record| state_name(record.state()).to_owned())
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
        vsh::TransactionState::Rejected => "rejected",
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

fn node_kind_name(kind: vsh::NodeKind) -> &'static str {
    match kind {
        vsh::NodeKind::File => "file",
        vsh::NodeKind::Directory => "directory",
        vsh::NodeKind::Symlink => "symlink",
    }
}

fn effect_origin_name(origin: vsh::EffectOrigin) -> &'static str {
    match origin {
        vsh::EffectOrigin::VirtualFs => "virtual_fs",
        vsh::EffectOrigin::MontyOsCall => "monty_os_call",
        vsh::EffectOrigin::MontyToolCall => "monty_tool_call",
        _ => "unknown",
    }
}

fn policy_profile_name(profile: vsh::PolicyProfile) -> &'static str {
    match profile {
        vsh::PolicyProfile::Balanced => "balanced",
        vsh::PolicyProfile::Strict => "strict",
        vsh::PolicyProfile::Paranoid => "paranoid",
    }
}

fn hook_verdict_name(verdict: vsh::HookVerdict) -> &'static str {
    match verdict {
        vsh::HookVerdict::FollowPolicy => "follow_policy",
        vsh::HookVerdict::Approve => "approve",
        vsh::HookVerdict::Review => "review",
        vsh::HookVerdict::Reject => "reject",
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
    module.add_class::<PyHookScope>()?;
    module.add_class::<PyReceiptDetail>()?;
    module.add_class::<PyExecutionBudget>()?;
    module.add_class::<PyRunRequest>()?;
    module.add_class::<PyNodeSummary>()?;
    module.add_class::<PyCanonicalChange>()?;
    module.add_class::<PyEffectSummary>()?;
    module.add_class::<PyReviewContent>()?;
    module.add_class::<PyRequestEvent>()?;
    module.add_class::<PyCommitPreparation>()?;
    module.add_class::<PyHookDecision>()?;
    module.add_class::<PyHookDecisionRecord>()?;
    module.add_class::<PyReceipt>()?;
    module.add_class::<PyCommitResolution>()?;
    module.add_class::<PyRecoveryReport>()?;
    module.add_class::<PyRuntime>()?;
    module.add_function(wrap_pyfunction!(version, module)?)?;
    module.add_function(wrap_pyfunction!(engine_kind, module)?)?;
    module.add_function(wrap_pyfunction!(normalize_path, module)?)?;
    Ok(())
}
