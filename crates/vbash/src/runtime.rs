use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use vsh_commit::{
    CommitConfig, CommitError, CommitPlan, CommitPlanError, CommitReceipt, Committer,
    RecoveryReport, SnapshotLimits,
};
use vsh_monty::{
    ExecutionError, ExecutionLimits, ExecutionOutcome, ExecutionStats, InProcessConfig,
    InProcessMonty, MontyObject, ResultCompatibility, ResultCompatibilityError, SubprocessConfig,
    SubprocessMonty, VirtualRoot, validate_result_compatibility,
};
use vsh_policy::{
    AccessKind, DeniedAccess, DenyManifest, PolicyDecision, PolicyInput, PolicyProfile,
    RiskManifest, RiskMetrics, TransactionIdentityInput, TransactionPolicy, bind_transaction,
};
use vsh_store::{
    ApprovalGrant, ApprovalGrantError, BlobStore, BlobStoreError, DataDirectory,
    DataDirectoryError, FileStoreConfig, FileTransactionStore, TransactionRecord, TransactionStore,
    TransactionStoreError,
};
use vsh_types::{
    DiffDigest, DiffEntry, RuntimeConfigDigest, SnapshotId, TransactionId, TransactionState,
};
use vsh_vfs::{CanonicalDiff, VfsError, VirtualFs};

use crate::artifact::{
    ArtifactError, PendingTransaction, ReviewEvidence, decode_pending, encode_pending,
};
use crate::hook::{
    CommitPreparation, CommitResolution, HookBaseline, HookConfig, HookDecision,
    HookDecisionRecord, HookHandlerError, HookVerdict, RequestEvent,
};

/// Request-scoped resource caps enforced by the Monty/VFS adapter.
pub type ExecutionBudget = ExecutionLimits;

/// Hard allocation and cardinality bounds for durable approval artifacts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactLimits {
    /// Maximum complete encoded artifact bytes.
    pub max_bytes: usize,
    /// Maximum postcard-encoded Monty result bytes.
    pub max_value_bytes: usize,
    /// Maximum retained UTF-8 stdout bytes.
    pub max_stdout_bytes: usize,
    /// Maximum canonical changed paths.
    pub max_entries: usize,
    /// Maximum read or write dependency entries.
    pub max_dependencies: usize,
    /// Maximum one-path UTF-8 byte length.
    pub max_path_bytes: usize,
    /// Maximum retained out-of-band intent bytes exposed to a hook.
    pub max_intent_bytes: usize,
    /// Maximum ordered operation-level effects exposed to a hook.
    pub max_effects: usize,
    /// Maximum process-local auto-approved previews retained by one runtime.
    pub max_ephemeral_entries: usize,
    /// Maximum encoded bytes retained by process-local auto-approved previews.
    pub max_ephemeral_bytes: usize,
}

impl Default for ArtifactLimits {
    fn default() -> Self {
        Self {
            max_bytes: 128 * 1024 * 1024,
            max_value_bytes: 16 * 1024 * 1024,
            max_stdout_bytes: 16 * 1024 * 1024,
            max_entries: 100_000,
            max_dependencies: 250_000,
            max_path_bytes: 16 * 1024,
            max_intent_bytes: 64 * 1024,
            max_effects: 250_000,
            max_ephemeral_entries: 64,
            max_ephemeral_bytes: 128 * 1024 * 1024,
        }
    }
}

/// Whether one call stops after policy or commits deterministic auto-approvals.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RunMode {
    /// Produce an approval-bound virtual result without changing the host workspace.
    #[default]
    Preview,
    /// Commit only when deterministic policy returns `AutoApprove`.
    Auto,
}

/// Amount of canonical change detail retained in a receipt.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ReceiptDetail {
    /// Retain bounded counts and digests, but no per-path diff entries.
    #[default]
    Compact,
    /// Retain the complete bounded canonical diff.
    Full,
}

/// One borrowed native execution request.
#[derive(Clone, Copy, Debug)]
pub struct RunRequest<'a> {
    /// Exact Monty source executed against virtual state.
    pub code: &'a str,
    /// Optional out-of-band intent bound into transaction identity.
    pub intent: Option<&'a str>,
    /// Preview-only or deterministic auto-commit behavior.
    pub mode: RunMode,
    /// Compact or complete canonical change detail.
    pub detail: ReceiptDetail,
    /// Independent execution caps for this request.
    pub budget: ExecutionBudget,
}

impl<'a> RunRequest<'a> {
    /// Construct a safe preview request with default resource caps.
    #[must_use]
    pub fn new(code: &'a str) -> Self {
        Self {
            code,
            intent: None,
            mode: RunMode::Preview,
            detail: ReceiptDetail::Compact,
            budget: ExecutionBudget::default(),
        }
    }

    /// Bind an out-of-band intent to this request.
    #[must_use]
    pub const fn with_intent(mut self, intent: &'a str) -> Self {
        self.intent = Some(intent);
        self
    }

    /// Select preview or deterministic auto-commit behavior.
    #[must_use]
    pub const fn with_mode(mut self, mode: RunMode) -> Self {
        self.mode = mode;
        self
    }

    /// Select compact or complete receipt detail.
    #[must_use]
    pub const fn with_detail(mut self, detail: ReceiptDetail) -> Self {
        self.detail = detail;
        self
    }

    /// Replace all request-scoped execution caps.
    #[must_use]
    pub const fn with_budget(mut self, budget: ExecutionBudget) -> Self {
        self.budget = budget;
        self
    }
}

/// Deterministic policy result retained in the native receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeDecision {
    /// Deterministic policy rejected the transaction.
    Denied(DenyManifest),
    /// Deterministic policy authorized reservation without a judge.
    AutoApproved,
    /// An exact independent approval is required before reservation.
    PendingApproval(RiskManifest),
}

/// Monotonic stage costs recorded without string allocation in the hot path.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StageTimings {
    /// Capability-rooted metadata snapshot time.
    pub snapshot_ns: u64,
    /// Monty execution plus typed VFS-call time.
    pub execute_ns: u64,
    /// Canonical-diff freeze time.
    pub diff_ns: u64,
    /// Deterministic-policy evaluation time.
    pub policy_ns: u64,
    /// Artifact binding and short state-store transitions.
    pub bind_and_store_ns: u64,
    /// Reservation, dependency revalidation, commit, and verification time.
    pub commit_ns: u64,
    /// Complete native call time.
    pub total_ns: u64,
}

/// Compact proof of virtual execution, policy, and optional verified commit.
#[derive(Clone, Debug)]
pub struct Receipt {
    /// Approval- and commit-bound transaction identity.
    pub transaction: TransactionId,
    /// Immutable base snapshot identity.
    pub base_snapshot: SnapshotId,
    /// Current lifecycle state. Auto-approved previews may be process-local until commit.
    pub state: TransactionState,
    /// Deterministic policy result.
    pub decision: RuntimeDecision,
    /// Canonical diff identity.
    pub diff: DiffDigest,
    /// Number of canonical changed paths.
    pub changed_paths: usize,
    /// Complete canonical entries only when full detail was requested.
    pub changes: Vec<DiffEntry>,
    /// Bounded Monty return value.
    pub value: MontyObject,
    /// Bounded captured `print()` output.
    pub stdout: String,
    /// Independent execution counters.
    pub execution: ExecutionStats,
    /// Native stage timings.
    pub timings: StageTimings,
    /// Durable commit proof when the host was changed and verified.
    pub commit: Option<CommitReceipt>,
}

/// Immutable runtime configuration shared by Rust and `PyO3` callers.
#[derive(Clone, Debug)]
pub struct RuntimeConfig {
    workspace_root: PathBuf,
    data_directory: PathBuf,
    data_directory_authority: DataDirectoryAuthority,
    worker_path: Option<PathBuf>,
    max_idle_workers: usize,
    result_compatibility: ResultCompatibility,
    virtual_root: VirtualRoot,
    policy: TransactionPolicy,
    snapshot_limits: SnapshotLimits,
    commit_config: CommitConfig,
    store_config: FileStoreConfig,
    artifact_limits: ArtifactLimits,
    commit_hook: Option<HookConfig>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DataDirectoryAuthority {
    WorkspaceProtected,
    TrustedExternal,
}

impl RuntimeConfig {
    /// Construct a balanced runtime rooted at `workspace_root`.
    ///
    /// Durable internal artifacts default to `.vsh-runtime/data` below the workspace;
    /// that namespace is excluded from snapshots and denied to Monty.
    #[must_use]
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        let workspace_root = workspace_root.into();
        let data_directory = workspace_root.join(".vsh-runtime").join("data");
        Self {
            workspace_root,
            data_directory,
            data_directory_authority: DataDirectoryAuthority::WorkspaceProtected,
            worker_path: Some(default_worker_path()),
            max_idle_workers: 4,
            result_compatibility: ResultCompatibility::Native,
            virtual_root: VirtualRoot::default(),
            policy: TransactionPolicy::default(),
            snapshot_limits: SnapshotLimits::default(),
            commit_config: CommitConfig::default(),
            store_config: FileStoreConfig::default(),
            artifact_limits: ArtifactLimits::default(),
            commit_hook: None,
        }
    }

    /// Place immutable blobs in an explicit trusted data directory.
    #[must_use]
    pub fn with_data_directory(mut self, data_directory: impl Into<PathBuf>) -> Self {
        self.data_directory = data_directory.into();
        self.data_directory_authority = DataDirectoryAuthority::TrustedExternal;
        self
    }

    /// Select the exact supervised Monty worker executable used for hostile code.
    #[must_use]
    pub fn with_worker_path(mut self, worker_path: impl Into<PathBuf>) -> Self {
        self.worker_path = Some(worker_path.into());
        self
    }

    /// Bound clean workers retained for low-latency reuse. Zero disables pooling.
    #[must_use]
    pub const fn with_max_idle_workers(mut self, max_idle_workers: usize) -> Self {
        self.max_idle_workers = max_idle_workers;
        self
    }

    /// Require every result to be faithfully representable by one host surface.
    #[must_use]
    pub const fn with_result_compatibility(
        mut self,
        result_compatibility: ResultCompatibility,
    ) -> Self {
        self.result_compatibility = result_compatibility;
        self
    }

    /// Disable crash isolation for trusted embedding and deterministic test harnesses.
    ///
    /// This mode must never execute hostile or unreviewed code. Production Rust and
    /// Python callers use the supervised worker by default.
    #[must_use]
    pub fn with_in_process_execution(mut self) -> Self {
        self.worker_path = None;
        self
    }

    /// Replace the synthetic absolute namespace exposed to Monty.
    #[must_use]
    pub fn with_virtual_root(mut self, virtual_root: VirtualRoot) -> Self {
        self.virtual_root = virtual_root;
        self
    }

    /// Replace deterministic transaction and pre-call policy.
    #[must_use]
    pub fn with_policy(mut self, policy: TransactionPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Select a built-in deterministic policy profile.
    #[must_use]
    pub fn with_policy_profile(self, profile: PolicyProfile) -> Self {
        self.with_policy(TransactionPolicy::preset(profile))
    }

    /// Replace eager snapshot traversal bounds.
    #[must_use]
    pub const fn with_snapshot_limits(mut self, limits: SnapshotLimits) -> Self {
        self.snapshot_limits = limits;
        self
    }

    /// Replace trusted commit and recovery bounds.
    #[must_use]
    pub const fn with_commit_config(mut self, config: CommitConfig) -> Self {
        self.commit_config = config;
        self
    }

    /// Replace durable transaction-log bounds.
    #[must_use]
    pub const fn with_store_config(mut self, config: FileStoreConfig) -> Self {
        self.store_config = config;
        self
    }

    /// Replace durable pending-artifact allocation and cardinality bounds.
    #[must_use]
    pub const fn with_artifact_limits(mut self, limits: ArtifactLimits) -> Self {
        self.artifact_limits = limits;
        self
    }

    /// Require the native two-phase hook protocol for matching commit candidates.
    #[must_use]
    pub const fn with_commit_hook(mut self, hook: HookConfig) -> Self {
        self.commit_hook = Some(hook);
        self
    }

    /// Return the host workspace authority root.
    #[must_use]
    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    /// Return the trusted immutable-artifact directory.
    #[must_use]
    pub fn data_directory(&self) -> &Path {
        &self.data_directory
    }

    /// Return the supervised worker path, or `None` for explicit trusted in-process mode.
    #[must_use]
    pub fn worker_path(&self) -> Option<&Path> {
        self.worker_path.as_deref()
    }

    /// Return deterministic transaction policy.
    #[must_use]
    pub const fn policy(&self) -> &TransactionPolicy {
        &self.policy
    }

    /// Return the commit-hook configuration, when direct commits are guarded.
    #[must_use]
    pub const fn commit_hook(&self) -> Option<HookConfig> {
        self.commit_hook
    }
}

fn default_worker_path() -> PathBuf {
    std::env::var_os("VSH_MONTY_WORKER")
        .filter(|path| !path.is_empty())
        .map_or_else(|| PathBuf::from("vsh-monty-worker"), PathBuf::from)
}

enum RuntimeExecution {
    Subprocess(Box<SubprocessMonty>),
    InProcess,
}

impl RuntimeExecution {
    fn open(config: &RuntimeConfig) -> Result<Self, ExecutionError> {
        let Some(worker_path) = &config.worker_path else {
            return Ok(Self::InProcess);
        };
        let adapter = InProcessConfig::new(config.virtual_root.clone())
            .with_call_policy(config.policy.call_policy().clone());
        let worker = SubprocessMonty::new(
            SubprocessConfig::new(worker_path, adapter)
                .with_max_idle_workers(config.max_idle_workers),
        )?;
        Ok(Self::Subprocess(Box::new(worker)))
    }

    fn security_digest(&self, adapter: &InProcessConfig) -> RuntimeConfigDigest {
        match self {
            Self::Subprocess(worker) => worker.config().security_digest_for(adapter),
            Self::InProcess => adapter.security_digest(),
        }
    }

    fn execute(
        &self,
        code: &str,
        filesystem: &mut VirtualFs,
        adapter: &InProcessConfig,
    ) -> Result<ExecutionOutcome, ExecutionError> {
        match self {
            Self::Subprocess(worker) => worker.execute_with_config(code, filesystem, adapter),
            Self::InProcess => InProcessMonty::new(adapter.clone()).execute(code, filesystem),
        }
    }
}

/// One native VSH engine instance with no process-global execution lock.
pub struct Runtime {
    config: RuntimeConfig,
    execution: RuntimeExecution,
    committer: Committer,
    store: FileTransactionStore,
    artifacts: BlobStore,
    pending: Mutex<PendingArtifacts>,
    startup_recovery: RecoveryReport,
}

#[derive(Default)]
struct PendingArtifacts {
    entries: BTreeMap<TransactionId, (PendingTransaction, usize)>,
    encoded_bytes: usize,
}

struct EvaluatedDiff {
    diff: CanonicalDiff,
    decision: PolicyDecision,
    metrics: RiskMetrics,
    diff_ns: u64,
    policy_ns: u64,
}

impl Runtime {
    /// Open one capability-rooted runtime and recover durable interrupted commits.
    ///
    /// # Errors
    ///
    /// Returns an error when blob storage, workspace capability setup, recovery, or
    /// fail-closed recovery conflict handling fails.
    pub fn open(config: RuntimeConfig) -> Result<Self, VshError> {
        let (committer, data_directory) = match config.data_directory_authority {
            DataDirectoryAuthority::WorkspaceProtected => {
                Committer::open_with_workspace_data(&config.workspace_root, config.commit_config)?
            }
            DataDirectoryAuthority::TrustedExternal => {
                validate_disjoint_data_directory(&config.workspace_root, &config.data_directory)?;
                let data_directory = DataDirectory::open_trusted(&config.data_directory)?;
                validate_canonical_data_directory_separation(
                    &config.workspace_root,
                    data_directory.path(),
                )?;
                let artifacts = BlobStore::open_in(&data_directory)?;
                let committer =
                    Committer::open(&config.workspace_root, artifacts, config.commit_config)?;
                (committer, data_directory)
            }
        };
        let artifacts = committer.artifact_store();
        let store = FileTransactionStore::open_in(&data_directory, config.store_config)?;
        let execution = RuntimeExecution::open(&config)?;
        let startup_recovery = committer.recover(&store)?;
        if !startup_recovery.conflicts.is_empty() {
            return Err(VshError::RecoveryConflicts(Box::new(startup_recovery)));
        }
        Ok(Self {
            config,
            execution,
            committer,
            store,
            artifacts,
            pending: Mutex::new(PendingArtifacts::default()),
            startup_recovery,
        })
    }

    /// Return the startup recovery work completed before accepting requests.
    #[must_use]
    pub const fn startup_recovery(&self) -> &RecoveryReport {
        &self.startup_recovery
    }

    /// Execute, evaluate, and optionally auto-commit one exact transaction.
    ///
    /// # Errors
    ///
    /// Returns a typed error for snapshot, execution, diff, state, binding, reservation,
    /// revalidation, commit, or recovery failures. Deterministic policy denial is a
    /// successful receipt and never reaches the committer.
    pub fn run(&self, request: RunRequest<'_>) -> Result<Receipt, VshError> {
        validate_program_size(request.code, request.budget)?;
        let total_started = Instant::now();
        let (mut filesystem, base_snapshot, base_node_count, snapshot_ns) =
            self.snapshot_filesystem()?;

        let monty_config = self.monty_config(request.budget);
        let runtime_config = self.runtime_config_digest(&monty_config);
        let execute_started = Instant::now();
        let ExecutionOutcome {
            value,
            stdout,
            stats,
            denied_accesses,
        } = self
            .execution
            .execute(request.code, &mut filesystem, &monty_config)?;
        validate_result_compatibility(&value, self.config.result_compatibility)?;
        let execute_ns = elapsed_ns(execute_started);

        let EvaluatedDiff {
            diff,
            decision: policy_decision,
            metrics: risk_metrics,
            diff_ns,
            policy_ns,
        } = self.evaluate_diff(&filesystem, &denied_accesses, base_node_count)?;

        let bind_started = Instant::now();
        let binding = bind_transaction(TransactionIdentityInput {
            base_snapshot,
            diff: &diff,
            read_set: filesystem.read_set(),
            write_set: filesystem.write_set(),
            program: request.code,
            policy: &self.config.policy,
            runtime_config,
            intent: request.intent,
        });
        let transaction = binding.transaction_id();
        let (decision, state, record) =
            Self::policy_record(transaction, base_snapshot, policy_decision)?;
        let changed_paths = diff.entries().len();
        let changes = receipt_changes(request.detail, &diff);
        let mut receipt = Receipt {
            transaction,
            base_snapshot,
            state,
            decision,
            diff: diff.digest(),
            changed_paths,
            changes,
            value,
            stdout,
            execution: stats,
            timings: StageTimings {
                snapshot_ns,
                execute_ns,
                diff_ns,
                policy_ns,
                bind_and_store_ns: elapsed_ns(bind_started),
                commit_ns: 0,
                total_ns: elapsed_ns(total_started),
            },
            commit: None,
        };

        if state == TransactionState::Denied {
            self.store.create(record)?;
            receipt.timings.bind_and_store_ns = elapsed_ns(bind_started);
            receipt.timings.total_ns = elapsed_ns(total_started);
        } else {
            receipt = self.store_pending(
                record,
                PendingTransaction {
                    binding,
                    diff,
                    read_set: filesystem.read_set().clone(),
                    write_set: filesystem.write_set().clone(),
                    review: ReviewEvidence {
                        intent: request.intent.map(str::to_owned),
                        metrics: risk_metrics,
                        effects: filesystem.effects().to_vec(),
                        complete: true,
                        truncated: false,
                    },
                    receipt,
                },
                request.mode,
                bind_started,
                total_started,
            )?;
        }

        if request.mode == RunMode::Auto && state == TransactionState::AutoApproved {
            receipt = self.commit(transaction, 0)?;
            receipt.timings.total_ns = elapsed_ns(total_started);
        }
        Ok(receipt)
    }

    /// Force preview-only behavior regardless of the request's mode field.
    ///
    /// # Errors
    ///
    /// Returns the same typed failures as [`Self::run`].
    pub fn preview(&self, mut request: RunRequest<'_>) -> Result<Receipt, VshError> {
        request.mode = RunMode::Preview;
        self.run(request)
    }

    /// Forget one process-local auto-approved preview without mutating the host.
    ///
    /// Durable approval-required artifacts are never removed by this method. `false`
    /// means this runtime did not retain the supplied preview.
    ///
    /// # Errors
    ///
    /// Returns an error only when the bounded pending-artifact lock was poisoned.
    pub fn discard_preview(&self, transaction: TransactionId) -> Result<bool, VshError> {
        self.remove_pending(transaction)
            .map(|artifact| artifact.is_some())
    }

    /// Bind an independent, expiring approval to one exact pending transaction.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid time window, missing transaction, mismatched
    /// binding, wrong state, or internal artifact-state mismatch.
    pub fn approve(
        &self,
        transaction: TransactionId,
        principal: vsh_types::PrincipalId,
        issued_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Result<TransactionRecord, VshError> {
        self.load_pending(transaction)?;
        let grant = ApprovalGrant::new(
            transaction,
            principal,
            issued_at_unix_ms,
            expires_at_unix_ms,
        )?;
        let record = self.store.approve(transaction, grant)?;
        if let Some((artifact, _)) = self.pending()?.entries.get_mut(&transaction) {
            artifact.receipt.state = TransactionState::Approved;
        }
        Ok(record)
    }

    /// Consume the single-use reservation and commit one previewed transaction.
    ///
    /// # Errors
    ///
    /// Returns an error for missing artifacts, expired approval, replay, stale host
    /// dependencies, commit/recovery failures, or internal binding mismatch.
    pub fn commit(
        &self,
        transaction: TransactionId,
        now_unix_ms: u64,
    ) -> Result<Receipt, VshError> {
        if self.config.commit_hook.is_some() {
            let preparation = self.prepare_commit(transaction)?;
            if let Some(event) = preparation.event() {
                return Err(VshError::HookRequired(Box::new(event.clone())));
            }
        }
        self.commit_exact(transaction, now_unix_ms)
    }

    /// Freeze one exact commit candidate before invoking an external handler.
    ///
    /// No handler code runs in this method. Process-local auto-approved previews are
    /// made durable before an event is returned, so a crash can regenerate the same
    /// event from the exact transaction artifact.
    ///
    /// # Errors
    ///
    /// Returns a typed store, artifact, configuration, or evidence error.
    pub fn prepare_commit(
        &self,
        transaction: TransactionId,
    ) -> Result<CommitPreparation, VshError> {
        let artifact = self.load_pending(transaction)?;
        validate_result_compatibility(&artifact.receipt.value, self.config.result_compatibility)?;
        self.persist_ephemeral(&artifact)?;
        let record = self.store.get(transaction)?;
        let state = record.state();
        let Some(hook) = self.config.commit_hook else {
            return Ok(CommitPreparation::Ready { transaction, state });
        };
        if !hook.scope().applies_to(state) {
            return Ok(CommitPreparation::Ready { transaction, state });
        }
        Ok(CommitPreparation::Review(Box::new(
            self.request_event(&artifact, hook, state)?,
        )))
    }

    /// Apply one hook decision to the exact prepared transaction.
    ///
    /// The preparation is revalidated against durable state and regenerated evidence,
    /// so callers cannot substitute a transaction or event after the handler returns.
    ///
    /// # Errors
    ///
    /// Returns a typed event-binding, state, approval, store, or commit error. The
    /// exact preparation is checked again before any decision changes host files.
    pub fn resolve_commit(
        &self,
        preparation: &CommitPreparation,
        decision: &HookDecision,
        now_unix_ms: u64,
    ) -> Result<CommitResolution, VshError> {
        let transaction = preparation.transaction();
        let prepared_state = preparation.prepared_state();
        let artifact = self.load_pending(transaction)?;
        let event = self.validate_hook_preparation(preparation, decision, &artifact)?;

        let (verdict, reason) = match &decision {
            HookDecision::FollowPolicy => (HookVerdict::FollowPolicy, ""),
            HookDecision::Approve { reason } => (HookVerdict::Approve, reason.as_str()),
            HookDecision::Review { feedback } => (HookVerdict::Review, feedback.as_str()),
            HookDecision::Reject { reason } => (HookVerdict::Reject, reason.as_str()),
        };
        if let Some((_, hook)) = event
            && reason.len() > hook.max_reason_bytes()
        {
            return Err(VshError::HookReasonLimit {
                observed: reason.len(),
                maximum: hook.max_reason_bytes(),
            });
        }
        let receipt = self.apply_hook_decision(
            transaction,
            prepared_state,
            &artifact,
            event,
            decision,
            now_unix_ms,
        )?;

        let hook_record = event.map(|(event, hook)| HookDecisionRecord {
            event_id: event.event_id,
            hook_id: event.hook_id,
            verdict,
            reason: reason.to_owned(),
            principal: (verdict == HookVerdict::Approve).then(|| hook.principal()),
        });
        Ok(CommitResolution {
            receipt,
            hook: hook_record,
        })
    }

    fn validate_hook_preparation<'a>(
        &self,
        preparation: &'a CommitPreparation,
        decision: &HookDecision,
        artifact: &PendingTransaction,
    ) -> Result<Option<(&'a RequestEvent, HookConfig)>, VshError> {
        let transaction = preparation.transaction();
        let prepared_state = preparation.prepared_state();
        let actual = self.store.get(transaction)?.state();
        if actual != prepared_state {
            return Err(VshError::HookStateChanged {
                transaction,
                prepared: prepared_state,
                actual,
            });
        }
        match preparation {
            CommitPreparation::Ready { .. } => {
                if let Some(hook) = self.config.commit_hook
                    && hook.scope().applies_to(prepared_state)
                {
                    return Err(VshError::HookRequired(Box::new(self.request_event(
                        artifact,
                        hook,
                        prepared_state,
                    )?)));
                }
                if *decision != HookDecision::FollowPolicy {
                    return Err(VshError::UnexpectedHookDecision { transaction });
                }
                Ok(None)
            }
            CommitPreparation::Review(event) => {
                let hook = self
                    .config
                    .commit_hook
                    .ok_or(VshError::HookConfigurationChanged { transaction })?;
                let expected = self.request_event(artifact, hook, prepared_state)?;
                if expected != **event {
                    return Err(VshError::HookEventMismatch { transaction });
                }
                Ok(Some((event.as_ref(), hook)))
            }
        }
    }

    fn apply_hook_decision(
        &self,
        transaction: TransactionId,
        prepared_state: TransactionState,
        artifact: &PendingTransaction,
        event: Option<(&RequestEvent, HookConfig)>,
        decision: &HookDecision,
        now_unix_ms: u64,
    ) -> Result<Receipt, VshError> {
        match decision {
            HookDecision::FollowPolicy => match prepared_state {
                TransactionState::AutoApproved | TransactionState::Approved => {
                    self.commit_exact(transaction, now_unix_ms)
                }
                TransactionState::PendingApproval => Ok(artifact.receipt.clone()),
                actual => Err(VshError::HookNotActionable {
                    transaction,
                    actual,
                }),
            },
            HookDecision::Approve { .. } => {
                let (_, hook) = event.ok_or(VshError::UnexpectedHookDecision { transaction })?;
                if !artifact.review.complete || artifact.review.truncated {
                    return Err(VshError::IncompleteHookEvidence { transaction });
                }
                if prepared_state == TransactionState::PendingApproval {
                    let expires_at_unix_ms = now_unix_ms
                        .checked_add(hook.approval_ttl_ms())
                        .ok_or(VshError::HookApprovalWindow { transaction })?;
                    self.approve(
                        transaction,
                        hook.principal(),
                        now_unix_ms,
                        expires_at_unix_ms,
                    )?;
                } else if prepared_state != TransactionState::AutoApproved {
                    return Err(VshError::HookNotActionable {
                        transaction,
                        actual: prepared_state,
                    });
                }
                self.commit_exact(transaction, now_unix_ms)
            }
            HookDecision::Review { .. } => {
                if prepared_state == TransactionState::AutoApproved {
                    self.store.compare_and_transition(
                        transaction,
                        TransactionState::AutoApproved,
                        TransactionState::PendingApproval,
                    )?;
                    self.update_pending_state(transaction, TransactionState::PendingApproval)?;
                } else if prepared_state != TransactionState::PendingApproval {
                    return Err(VshError::HookNotActionable {
                        transaction,
                        actual: prepared_state,
                    });
                }
                self.receipt_in_state(transaction, TransactionState::PendingApproval)
            }
            HookDecision::Reject { .. } => {
                if !matches!(
                    prepared_state,
                    TransactionState::AutoApproved | TransactionState::PendingApproval
                ) {
                    return Err(VshError::HookNotActionable {
                        transaction,
                        actual: prepared_state,
                    });
                }
                self.store.compare_and_transition(
                    transaction,
                    prepared_state,
                    TransactionState::Rejected,
                )?;
                self.update_pending_state(transaction, TransactionState::Rejected)?;
                let receipt = self.receipt_in_state(transaction, TransactionState::Rejected)?;
                self.remove_pending(transaction)?;
                Ok(receipt)
            }
        }
    }

    /// Apply fail-closed state after a handler exception, timeout, or cancellation.
    ///
    /// # Errors
    ///
    /// Returns a typed store or artifact error if automatic approval cannot be moved
    /// into the existing pending-approval state.
    pub fn fail_hook(&self, preparation: &CommitPreparation) -> Result<(), VshError> {
        let transaction = preparation.transaction();
        if preparation.prepared_state() == TransactionState::AutoApproved {
            self.store.compare_and_transition(
                transaction,
                TransactionState::AutoApproved,
                TransactionState::PendingApproval,
            )?;
            self.update_pending_state(transaction, TransactionState::PendingApproval)?;
        }
        Ok(())
    }

    fn commit_exact(
        &self,
        transaction: TransactionId,
        now_unix_ms: u64,
    ) -> Result<Receipt, VshError> {
        let artifact = self.load_pending(transaction)?;
        validate_result_compatibility(&artifact.receipt.value, self.config.result_compatibility)?;
        self.persist_ephemeral(&artifact)?;
        let plan = CommitPlan::new(
            &artifact.binding,
            &artifact.diff,
            &artifact.read_set,
            &artifact.write_set,
        )?;
        let reservation = self.store.reserve(transaction, now_unix_ms)?;
        let commit_started = Instant::now();
        let commit = self.committer.commit(&self.store, reservation, &plan);
        let commit_ns = elapsed_ns(commit_started);
        self.remove_pending(transaction)?;
        let commit = commit?;
        let mut receipt = artifact.receipt;
        receipt.state = TransactionState::Committed;
        receipt.timings.commit_ns = commit_ns;
        receipt.timings.total_ns = receipt.timings.total_ns.saturating_add(commit_ns);
        receipt.commit = Some(commit);
        Ok(receipt)
    }

    /// Recover all durable commit artifacts under this runtime's capability root.
    ///
    /// # Errors
    ///
    /// Returns a typed commit/recovery error for corrupt or unsafe journals.
    pub fn recover(&self) -> Result<RecoveryReport, VshError> {
        self.committer.recover(&self.store).map_err(Into::into)
    }

    /// Return one persisted lifecycle record.
    ///
    /// # Errors
    ///
    /// Returns [`VshError::Store`] when the transaction does not exist.
    pub fn transaction(&self, transaction: TransactionId) -> Result<TransactionRecord, VshError> {
        match self.store.get(transaction) {
            Ok(record) => Ok(record),
            Err(TransactionStoreError::NotFound { id }) if id == transaction => {
                let artifact = self
                    .pending()?
                    .entries
                    .get(&transaction)
                    .map(|(artifact, _)| artifact.clone())
                    .ok_or(TransactionStoreError::NotFound { id })?;
                Self::ephemeral_record(&artifact)
            }
            Err(source) => Err(source.into()),
        }
    }

    fn request_event(
        &self,
        artifact: &PendingTransaction,
        hook: HookConfig,
        state: TransactionState,
    ) -> Result<RequestEvent, VshError> {
        let transaction = artifact.binding.transaction_id();
        if artifact.binding.policy != self.config.policy.digest() {
            return Err(VshError::HookConfigurationChanged { transaction });
        }
        let (baseline, risk_flags) = match &artifact.receipt.decision {
            RuntimeDecision::AutoApproved => (HookBaseline::AutoApproved, Vec::new()),
            RuntimeDecision::PendingApproval(manifest) => {
                (HookBaseline::ReviewRequired, manifest.flags.clone())
            }
            RuntimeDecision::Denied(_) => {
                return Err(VshError::HookNotActionable {
                    transaction,
                    actual: TransactionState::Denied,
                });
            }
        };
        let (contents, content_complete) = crate::review::collect_content(
            artifact.diff.entries(),
            &artifact.review.effects,
            self.config.policy.call_policy(),
            &self.artifacts,
            hook.max_content_bytes(),
        )?;
        Ok(RequestEvent {
            schema_version: 1,
            event_id: vsh_types::RequestEventId::derive(transaction, hook.id(), hook.scope().tag()),
            hook_id: hook.id(),
            transaction,
            state,
            baseline,
            base_snapshot: artifact.binding.base_snapshot,
            diff: artifact.binding.diff,
            read_set: artifact.binding.read_set,
            write_set: artifact.binding.write_set,
            program: artifact.binding.program,
            policy: artifact.binding.policy,
            runtime_config: artifact.binding.runtime_config,
            intent_digest: artifact.binding.intent,
            intent: artifact.review.intent.clone(),
            policy_profile: self.config.policy.profile(),
            policy_thresholds: self.config.policy.thresholds(),
            risk_metrics: artifact.review.metrics,
            risk_flags,
            canonical_diff: artifact.diff.entries().to_vec(),
            effects: artifact.review.effects.clone(),
            execution: artifact.receipt.execution,
            evidence_complete: artifact.review.complete,
            evidence_truncated: artifact.review.truncated,
            contents,
            content_complete,
        })
    }

    fn evaluate_diff(
        &self,
        filesystem: &VirtualFs,
        denied_accesses: &[DeniedAccess],
        base_node_count: usize,
    ) -> Result<EvaluatedDiff, VshError> {
        let started = Instant::now();
        let mut diff = filesystem.canonical_diff()?;
        let mut diff_ns = elapsed_ns(started);
        let evaluate = |diff: &CanonicalDiff| {
            self.config.policy.evaluate_with_metrics(PolicyInput {
                diff,
                effects: filesystem.effects(),
                denied_accesses,
                base_node_count,
            })
        };
        let started = Instant::now();
        let (mut decision, mut metrics) = evaluate(&diff);
        let mut policy_ns = elapsed_ns(started);
        let state = match &decision {
            PolicyDecision::Deny(_) => TransactionState::Denied,
            PolicyDecision::AutoApprove => TransactionState::AutoApproved,
            PolicyDecision::Escalate(_) => TransactionState::PendingApproval,
        };
        if let Some(hook) = self.config.commit_hook
            && hook.max_content_bytes() > 0
            && hook.scope().applies_to(state)
            && !diff.entries().is_empty()
        {
            let started = Instant::now();
            let paths = diff.entries().iter().filter_map(|entry| {
                (entry.before.is_some()
                    && self
                        .config
                        .policy
                        .call_policy()
                        .authorize(&entry.path, AccessKind::ContentRead)
                        .is_ok())
                .then_some(&entry.path)
            });
            filesystem.capture_before_content(paths, hook.max_content_bytes())?;
            diff = filesystem.canonical_diff()?;
            diff_ns = diff_ns.saturating_add(elapsed_ns(started));
            let started = Instant::now();
            (decision, metrics) = evaluate(&diff);
            policy_ns = policy_ns.saturating_add(elapsed_ns(started));
        }
        Ok(EvaluatedDiff {
            diff,
            decision,
            metrics,
            diff_ns,
            policy_ns,
        })
    }

    fn update_pending_state(
        &self,
        transaction: TransactionId,
        state: TransactionState,
    ) -> Result<(), VshError> {
        if let Some((artifact, _)) = self.pending()?.entries.get_mut(&transaction) {
            artifact.receipt.state = state;
        }
        Ok(())
    }

    fn receipt_in_state(
        &self,
        transaction: TransactionId,
        state: TransactionState,
    ) -> Result<Receipt, VshError> {
        let mut artifact = self.load_pending(transaction)?;
        artifact.receipt.state = state;
        Ok(artifact.receipt)
    }

    fn monty_config(&self, budget: ExecutionBudget) -> InProcessConfig {
        InProcessConfig::new(self.config.virtual_root.clone())
            .with_limits(budget)
            .with_call_policy(self.config.policy.call_policy().clone())
    }

    fn runtime_config_digest(&self, monty_config: &InProcessConfig) -> RuntimeConfigDigest {
        aggregate_runtime_digest(
            self.execution.security_digest(monty_config),
            self.config.snapshot_limits,
            self.config.commit_config,
            self.config.store_config,
            self.config.artifact_limits,
            self.config.result_compatibility,
            self.config.commit_hook,
        )
    }

    fn snapshot_filesystem(&self) -> Result<(VirtualFs, SnapshotId, usize, u64), VshError> {
        let started = Instant::now();
        let snapshot = self.committer.snapshot(self.config.snapshot_limits)?;
        let id = snapshot.id();
        let nodes = snapshot.len();
        Ok((VirtualFs::new(snapshot), id, nodes, elapsed_ns(started)))
    }

    fn insert_pending(
        &self,
        artifact: PendingTransaction,
        encoded_bytes: usize,
    ) -> Result<(), VshError> {
        let transaction = artifact.binding.transaction_id();
        let mut pending = self.pending()?;
        let entries = pending.entries.len();
        let retained_bytes = pending.encoded_bytes;
        let attempted_bytes = retained_bytes.saturating_add(encoded_bytes);
        if entries >= self.config.artifact_limits.max_ephemeral_entries
            || attempted_bytes > self.config.artifact_limits.max_ephemeral_bytes
        {
            return Err(VshError::EphemeralCapacity {
                entries,
                max_entries: self.config.artifact_limits.max_ephemeral_entries,
                attempted_bytes,
                max_bytes: self.config.artifact_limits.max_ephemeral_bytes,
            });
        }
        if pending.entries.contains_key(&transaction) {
            return Err(VshError::DuplicatePending { transaction });
        }
        pending
            .entries
            .insert(transaction, (artifact, encoded_bytes));
        pending.encoded_bytes = attempted_bytes;
        Ok(())
    }

    fn remove_pending(
        &self,
        transaction: TransactionId,
    ) -> Result<Option<PendingTransaction>, VshError> {
        let mut pending = self.pending()?;
        let Some((artifact, encoded_bytes)) = pending.entries.remove(&transaction) else {
            return Ok(None);
        };
        pending.encoded_bytes = pending.encoded_bytes.saturating_sub(encoded_bytes);
        Ok(Some(artifact))
    }

    fn persist_pending(
        &self,
        record: TransactionRecord,
        mut artifact: PendingTransaction,
        bind_started: Instant,
        total_started: Instant,
    ) -> Result<Receipt, VshError> {
        let encoded = encode_pending(&artifact, self.config.artifact_limits)?;
        let artifact_id = self.artifacts.put(&encoded)?;
        self.store.create(record.with_artifact(artifact_id))?;
        artifact.receipt.timings.bind_and_store_ns = elapsed_ns(bind_started);
        artifact.receipt.timings.total_ns = elapsed_ns(total_started);
        let receipt = artifact.receipt.clone();
        Ok(receipt)
    }

    fn store_pending(
        &self,
        record: TransactionRecord,
        artifact: PendingTransaction,
        mode: RunMode,
        bind_started: Instant,
        total_started: Instant,
    ) -> Result<Receipt, VshError> {
        if mode == RunMode::Preview && artifact.receipt.state == TransactionState::AutoApproved {
            self.retain_ephemeral(artifact, bind_started, total_started)
        } else {
            self.persist_pending(record, artifact, bind_started, total_started)
        }
    }

    fn retain_ephemeral(
        &self,
        mut artifact: PendingTransaction,
        bind_started: Instant,
        total_started: Instant,
    ) -> Result<Receipt, VshError> {
        let encoded = encode_pending(&artifact, self.config.artifact_limits)?;
        artifact.receipt.timings.bind_and_store_ns = elapsed_ns(bind_started);
        artifact.receipt.timings.total_ns = elapsed_ns(total_started);
        let receipt = artifact.receipt.clone();
        self.insert_pending(artifact, encoded.len())?;
        Ok(receipt)
    }

    fn persist_ephemeral(&self, artifact: &PendingTransaction) -> Result<(), VshError> {
        let transaction = artifact.binding.transaction_id();
        match self.store.get(transaction) {
            Ok(_) => return Ok(()),
            Err(TransactionStoreError::NotFound { id }) if id == transaction => {}
            Err(source) => return Err(source.into()),
        }
        let encoded = encode_pending(artifact, self.config.artifact_limits)?;
        let artifact_id = self.artifacts.put(&encoded)?;
        let record = Self::ephemeral_record(artifact)?.with_artifact(artifact_id);
        self.store.create(record)?;
        Ok(())
    }

    fn ephemeral_record(artifact: &PendingTransaction) -> Result<TransactionRecord, VshError> {
        if artifact.receipt.state != TransactionState::AutoApproved
            || !matches!(&artifact.receipt.decision, RuntimeDecision::AutoApproved)
        {
            return Err(VshError::MissingPending {
                transaction: artifact.binding.transaction_id(),
            });
        }
        let mut record = TransactionRecord::new(
            artifact.binding.transaction_id(),
            artifact.binding.base_snapshot,
        );
        for state in [
            TransactionState::Running,
            TransactionState::VirtualComplete,
            TransactionState::AutoApproved,
        ] {
            record
                .transition(state)
                .map_err(TransactionStoreError::Transition)?;
        }
        Ok(record)
    }

    fn load_pending(&self, transaction: TransactionId) -> Result<PendingTransaction, VshError> {
        if let Some(artifact) = self
            .pending()?
            .entries
            .get(&transaction)
            .map(|(artifact, _)| artifact.clone())
        {
            return Ok(artifact);
        }
        let record = self.store.get(transaction)?;
        let artifact_id = record
            .artifact()
            .ok_or(VshError::MissingPending { transaction })?;
        let bytes = self
            .artifacts
            .get_bounded(artifact_id, self.config.artifact_limits.max_bytes)?;
        let mut artifact = decode_pending(&bytes, self.config.artifact_limits)?;
        let actual = artifact.binding.transaction_id();
        if actual != transaction || artifact.binding.base_snapshot != record.base_snapshot() {
            return Err(VshError::ArtifactBinding {
                requested: transaction,
                decoded: actual,
            });
        }
        artifact.receipt.state = record.state();
        Ok(artifact)
    }

    fn pending(&self) -> Result<MutexGuard<'_, PendingArtifacts>, VshError> {
        self.pending.lock().map_err(|_| VshError::PendingPoisoned)
    }

    fn policy_record(
        transaction: TransactionId,
        base_snapshot: SnapshotId,
        decision: PolicyDecision,
    ) -> Result<(RuntimeDecision, TransactionState, TransactionRecord), VshError> {
        let mut record = TransactionRecord::new(transaction, base_snapshot);
        record
            .transition(TransactionState::Running)
            .map_err(TransactionStoreError::Transition)?;
        record
            .transition(TransactionState::VirtualComplete)
            .map_err(TransactionStoreError::Transition)?;
        let (decision, state) = match decision {
            PolicyDecision::Deny(manifest) => {
                record
                    .transition(TransactionState::Denied)
                    .map_err(TransactionStoreError::Transition)?;
                (RuntimeDecision::Denied(manifest), TransactionState::Denied)
            }
            PolicyDecision::AutoApprove => {
                record
                    .transition(TransactionState::AutoApproved)
                    .map_err(TransactionStoreError::Transition)?;
                (
                    RuntimeDecision::AutoApproved,
                    TransactionState::AutoApproved,
                )
            }
            PolicyDecision::Escalate(manifest) => {
                record
                    .transition(TransactionState::PendingApproval)
                    .map_err(TransactionStoreError::Transition)?;
                (
                    RuntimeDecision::PendingApproval(manifest),
                    TransactionState::PendingApproval,
                )
            }
        };
        Ok((decision, state, record))
    }
}

fn aggregate_runtime_digest(
    monty: RuntimeConfigDigest,
    snapshot: SnapshotLimits,
    commit: CommitConfig,
    store: FileStoreConfig,
    artifact: ArtifactLimits,
    result_compatibility: ResultCompatibility,
    commit_hook: Option<HookConfig>,
) -> RuntimeConfigDigest {
    let mut canonical = Vec::with_capacity(66 + 8 * 23);
    canonical.extend_from_slice(b"vsh-runtime-config-v6");
    canonical.extend_from_slice(monty.as_bytes());
    encode_usize(snapshot.max_nodes, &mut canonical);
    encode_usize(snapshot.max_depth, &mut canonical);
    canonical.extend_from_slice(&snapshot.max_total_file_bytes.to_le_bytes());
    encode_usize(commit.max_operations, &mut canonical);
    encode_usize(commit.max_dependencies, &mut canonical);
    encode_usize(commit.max_path_bytes, &mut canonical);
    encode_usize(commit.max_plan_bytes, &mut canonical);
    encode_usize(commit.max_journal_bytes, &mut canonical);
    encode_usize(commit.max_conflicts, &mut canonical);
    canonical.extend_from_slice(&store.max_log_bytes.to_le_bytes());
    encode_usize(store.max_records, &mut canonical);
    encode_usize(artifact.max_bytes, &mut canonical);
    encode_usize(artifact.max_value_bytes, &mut canonical);
    encode_usize(artifact.max_stdout_bytes, &mut canonical);
    encode_usize(artifact.max_entries, &mut canonical);
    encode_usize(artifact.max_dependencies, &mut canonical);
    encode_usize(artifact.max_path_bytes, &mut canonical);
    encode_usize(artifact.max_intent_bytes, &mut canonical);
    encode_usize(artifact.max_effects, &mut canonical);
    encode_usize(artifact.max_ephemeral_entries, &mut canonical);
    encode_usize(artifact.max_ephemeral_bytes, &mut canonical);
    canonical.push(match result_compatibility {
        ResultCompatibility::Native => 0,
        ResultCompatibility::Python => 1,
    });
    match commit_hook {
        None => canonical.push(0),
        Some(hook) => {
            canonical.push(1);
            canonical.extend_from_slice(hook.id().as_bytes());
            canonical.push(hook.scope().tag());
            canonical.extend_from_slice(&hook.approval_ttl_ms().to_le_bytes());
            encode_usize(hook.max_reason_bytes(), &mut canonical);
            encode_usize(hook.max_content_bytes(), &mut canonical);
        }
    }
    RuntimeConfigDigest::digest_canonical(&canonical)
}

fn encode_usize(value: usize, output: &mut Vec<u8>) {
    output.extend_from_slice(&u64::try_from(value).unwrap_or(u64::MAX).to_le_bytes());
}

fn receipt_changes(detail: ReceiptDetail, diff: &CanonicalDiff) -> Vec<DiffEntry> {
    if detail == ReceiptDetail::Full {
        diff.entries().to_vec()
    } else {
        Vec::new()
    }
}

fn elapsed_ns(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn validate_program_size(code: &str, budget: ExecutionBudget) -> Result<(), ExecutionError> {
    let attempted = u64::try_from(code.len()).unwrap_or(u64::MAX);
    let limit = u64::try_from(budget.max_program_bytes).unwrap_or(u64::MAX);
    if attempted > limit {
        Err(ExecutionError::Limit(Box::new(
            vsh_monty::ExecutionLimitExceeded::ProgramBytes { limit, attempted },
        )))
    } else {
        Ok(())
    }
}

fn validate_disjoint_data_directory(
    workspace_root: &Path,
    data_directory: &Path,
) -> Result<(), VshError> {
    let workspace = lexical_absolute(workspace_root);
    let data = lexical_absolute(data_directory);
    let canonical_workspace = std::fs::canonicalize(workspace_root).ok();
    let prospective_data = canonicalize_prospective_path(data_directory);
    if workspace
        .as_deref()
        .zip(data.as_deref())
        .is_none_or(|(workspace, data)| paths_overlap(workspace, data))
        || canonical_workspace
            .as_deref()
            .zip(prospective_data.as_deref())
            .is_none_or(|(workspace, data)| paths_overlap(workspace, data))
    {
        return Err(VshError::UnsafeDataDirectory {
            workspace_root: workspace_root.to_path_buf(),
            data_directory: data_directory.to_path_buf(),
        });
    }
    Ok(())
}

fn validate_canonical_data_directory_separation(
    workspace_root: &Path,
    data_directory: &Path,
) -> Result<(), VshError> {
    let Ok(workspace) = std::fs::canonicalize(workspace_root) else {
        return Err(VshError::UnsafeDataDirectory {
            workspace_root: workspace_root.to_path_buf(),
            data_directory: data_directory.to_path_buf(),
        });
    };
    let Ok(data) = std::fs::canonicalize(data_directory) else {
        return Err(VshError::UnsafeDataDirectory {
            workspace_root: workspace_root.to_path_buf(),
            data_directory: data_directory.to_path_buf(),
        });
    };
    if paths_overlap(&workspace, &data) {
        return Err(VshError::UnsafeDataDirectory {
            workspace_root: workspace_root.to_path_buf(),
            data_directory: data_directory.to_path_buf(),
        });
    }
    Ok(())
}

fn canonicalize_prospective_path(path: &Path) -> Option<PathBuf> {
    let absolute = lexical_absolute(path)?;
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(existing) {
            Ok(mut canonical) => {
                for component in missing.iter().rev() {
                    canonical.push(component);
                }
                return Some(canonical);
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                missing.push(existing.file_name()?.to_owned());
                existing = existing.parent()?;
            }
            Err(_) => return None,
        }
    }
}

fn lexical_absolute(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    Some(normalized)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

/// Stable native error surface shared with the Python exception mapper.
#[derive(Debug)]
#[non_exhaustive]
pub enum VshError {
    /// The durable data-directory capability could not be established safely.
    DataDirectory(DataDirectoryError),
    /// Immutable blob storage failed.
    Blob(BlobStoreError),
    /// Capability-rooted commit or recovery failed.
    Commit(CommitError),
    /// Monty compilation, execution, or a hard execution budget failed.
    Execution(ExecutionError),
    /// Virtual filesystem integrity or canonical diff generation failed.
    Vfs(VfsError),
    /// Atomic transaction-state operation failed.
    Store(TransactionStoreError),
    /// An approval grant had an invalid time window.
    Approval(ApprovalGrantError),
    /// The internal commit artifact did not match its binding.
    CommitPlan(CommitPlanError),
    /// Durable pending-artifact encoding or validation failed.
    Artifact(ArtifactError),
    /// The selected SDK surface cannot faithfully project the Monty result.
    ResultCompatibility(ResultCompatibilityError),
    /// A caller-selected data directory overlaps the untrusted workspace.
    UnsafeDataDirectory {
        /// Host workspace capability root.
        workspace_root: PathBuf,
        /// Rejected caller-selected durable directory.
        data_directory: PathBuf,
    },
    /// A content-addressed artifact decoded to another transaction identity.
    ArtifactBinding {
        /// Transaction requested by the caller and state store.
        requested: TransactionId,
        /// Transaction recomputed from decoded artifact contents.
        decoded: TransactionId,
    },
    /// Startup recovery found ownership it could not prove and left it untouched.
    RecoveryConflicts(Box<RecoveryReport>),
    /// The transaction record has no durable exact artifact.
    MissingPending {
        /// Requested transaction.
        transaction: TransactionId,
    },
    /// A duplicate exact artifact attempted to occupy the pending map.
    DuplicatePending {
        /// Duplicate transaction.
        transaction: TransactionId,
    },
    /// Process-local preview retention reached its configured hard bound.
    EphemeralCapacity {
        /// Number of previews retained before this attempt.
        entries: usize,
        /// Maximum previews retained by one runtime.
        max_entries: usize,
        /// Total encoded bytes that retaining this preview would require.
        attempted_bytes: usize,
        /// Maximum encoded bytes retained by one runtime.
        max_bytes: usize,
    },
    /// The short-lived pending-artifact mutex was poisoned by a panic.
    PendingPoisoned,
    /// Direct commit was blocked because a configured hook must decide first.
    HookRequired(Box<RequestEvent>),
    /// The external hook handler failed after fail-closed state was applied.
    HookHandler(HookHandlerError),
    /// Durable state changed after the event was prepared.
    HookStateChanged {
        /// Exact transaction represented by the preparation.
        transaction: TransactionId,
        /// State captured while preparing the hook event.
        prepared: TransactionState,
        /// State observed when resolving the hook decision.
        actual: TransactionState,
    },
    /// Hook configuration no longer matches the prepared transaction evidence.
    HookConfigurationChanged {
        /// Transaction whose bound hook configuration changed.
        transaction: TransactionId,
    },
    /// The event supplied for resolution was not the exact regenerated event.
    HookEventMismatch {
        /// Transaction whose regenerated event did not match.
        transaction: TransactionId,
    },
    /// A handler decision was supplied when no handler was requested.
    UnexpectedHookDecision {
        /// Transaction that did not request a hook decision.
        transaction: TransactionId,
    },
    /// The selected transaction state cannot accept a hook decision.
    HookNotActionable {
        /// Transaction rejected by the hook state guard.
        transaction: TransactionId,
        /// Non-actionable state observed by the resolver.
        actual: TransactionState,
    },
    /// Hook feedback exceeded its configured hard UTF-8 bound.
    HookReasonLimit {
        /// Observed UTF-8 byte length.
        observed: usize,
        /// Configured maximum UTF-8 byte length.
        maximum: usize,
    },
    /// Hook approval expiry overflowed or did not advance host time.
    HookApprovalWindow {
        /// Transaction whose approval window was invalid.
        transaction: TransactionId,
    },
    /// Legacy or truncated evidence cannot be approved by an automated hook.
    IncompleteHookEvidence {
        /// Transaction without complete hook evidence.
        transaction: TransactionId,
    },
}

impl fmt::Display for VshError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DataDirectory(source) => fmt::Display::fmt(source, formatter),
            Self::Blob(source) => fmt::Display::fmt(source, formatter),
            Self::Commit(source) => fmt::Display::fmt(source, formatter),
            Self::Execution(source) => fmt::Display::fmt(source, formatter),
            Self::Vfs(source) => fmt::Display::fmt(source, formatter),
            Self::Store(source) => fmt::Display::fmt(source, formatter),
            Self::Approval(source) => fmt::Display::fmt(source, formatter),
            Self::CommitPlan(source) => fmt::Display::fmt(source, formatter),
            Self::Artifact(source) => fmt::Display::fmt(source, formatter),
            Self::ResultCompatibility(source) => fmt::Display::fmt(source, formatter),
            Self::UnsafeDataDirectory {
                workspace_root,
                data_directory,
            } => write!(
                formatter,
                "trusted data directory {} must be disjoint from workspace {}",
                data_directory.display(),
                workspace_root.display()
            ),
            Self::ArtifactBinding { requested, decoded } => write!(
                formatter,
                "pending artifact for {requested} decodes to transaction {decoded}"
            ),
            Self::RecoveryConflicts(report) => write!(
                formatter,
                "startup recovery left {} ambiguous transaction(s)",
                report.conflicts.len()
            ),
            Self::MissingPending { transaction } => {
                write!(
                    formatter,
                    "no durable pending artifact for transaction {transaction}"
                )
            }
            Self::DuplicatePending { transaction } => {
                write!(
                    formatter,
                    "pending artifact already exists for {transaction}"
                )
            }
            Self::EphemeralCapacity {
                entries,
                max_entries,
                attempted_bytes,
                max_bytes,
            } => write!(
                formatter,
                "process-local preview capacity exceeded: {entries}/{max_entries} entries, \
                 {attempted_bytes}/{max_bytes} encoded bytes"
            ),
            Self::PendingPoisoned => formatter.write_str("pending artifact lock was poisoned"),
            Self::HookRequired(event) => write!(
                formatter,
                "commit hook {} must decide request event {} for transaction {}",
                event.hook_id, event.event_id, event.transaction
            ),
            Self::HookHandler(source) => write!(formatter, "commit hook handler failed: {source}"),
            Self::HookStateChanged {
                transaction,
                prepared,
                actual,
            } => write!(
                formatter,
                "transaction {transaction} changed from prepared state {prepared:?} to {actual:?}"
            ),
            Self::HookConfigurationChanged { transaction } => write!(
                formatter,
                "commit hook configuration changed for transaction {transaction}"
            ),
            Self::HookEventMismatch { transaction } => write!(
                formatter,
                "commit hook event does not match transaction {transaction}"
            ),
            Self::UnexpectedHookDecision { transaction } => write!(
                formatter,
                "transaction {transaction} did not request a hook decision"
            ),
            Self::HookNotActionable {
                transaction,
                actual,
            } => write!(
                formatter,
                "transaction {transaction} in state {actual:?} cannot accept a hook decision"
            ),
            Self::HookReasonLimit { observed, maximum } => write!(
                formatter,
                "hook feedback uses {observed} bytes, exceeding the {maximum}-byte limit"
            ),
            Self::HookApprovalWindow { transaction } => write!(
                formatter,
                "hook approval window is invalid for transaction {transaction}"
            ),
            Self::IncompleteHookEvidence { transaction } => write!(
                formatter,
                "transaction {transaction} has incomplete evidence and cannot be hook-approved"
            ),
        }
    }
}

impl Error for VshError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DataDirectory(source) => Some(source),
            Self::Blob(source) => Some(source),
            Self::Commit(source) => Some(source),
            Self::Execution(source) => Some(source),
            Self::Vfs(source) => Some(source),
            Self::Store(source) => Some(source),
            Self::Approval(source) => Some(source),
            Self::CommitPlan(source) => Some(source),
            Self::Artifact(source) => Some(source),
            Self::ResultCompatibility(source) => Some(source),
            Self::HookHandler(source) => Some(source),
            Self::RecoveryConflicts(_)
            | Self::UnsafeDataDirectory { .. }
            | Self::ArtifactBinding { .. }
            | Self::MissingPending { .. }
            | Self::DuplicatePending { .. }
            | Self::EphemeralCapacity { .. }
            | Self::PendingPoisoned
            | Self::HookRequired(_)
            | Self::HookStateChanged { .. }
            | Self::HookConfigurationChanged { .. }
            | Self::HookEventMismatch { .. }
            | Self::UnexpectedHookDecision { .. }
            | Self::HookNotActionable { .. }
            | Self::HookReasonLimit { .. }
            | Self::HookApprovalWindow { .. }
            | Self::IncompleteHookEvidence { .. } => None,
        }
    }
}

impl From<DataDirectoryError> for VshError {
    fn from(source: DataDirectoryError) -> Self {
        Self::DataDirectory(source)
    }
}

impl From<BlobStoreError> for VshError {
    fn from(source: BlobStoreError) -> Self {
        Self::Blob(source)
    }
}

impl From<CommitError> for VshError {
    fn from(source: CommitError) -> Self {
        Self::Commit(source)
    }
}

impl From<ExecutionError> for VshError {
    fn from(source: ExecutionError) -> Self {
        Self::Execution(source)
    }
}

impl From<VfsError> for VshError {
    fn from(source: VfsError) -> Self {
        Self::Vfs(source)
    }
}

impl From<TransactionStoreError> for VshError {
    fn from(source: TransactionStoreError) -> Self {
        Self::Store(source)
    }
}

impl From<ApprovalGrantError> for VshError {
    fn from(source: ApprovalGrantError) -> Self {
        Self::Approval(source)
    }
}

impl From<CommitPlanError> for VshError {
    fn from(source: CommitPlanError) -> Self {
        Self::CommitPlan(source)
    }
}

impl From<ArtifactError> for VshError {
    fn from(source: ArtifactError) -> Self {
        Self::Artifact(source)
    }
}

impl From<ResultCompatibilityError> for VshError {
    fn from(source: ResultCompatibilityError) -> Self {
        Self::ResultCompatibility(source)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use vsh_commit::CommitError;
    use vsh_policy::{DenyReason, PolicyProfile};
    use vsh_types::{PrincipalId, TransactionState};

    use super::{
        ApprovalGrantError, ArtifactError, ArtifactLimits, BlobStoreError, CommitPlanError,
        DataDirectory, ExecutionBudget, ExecutionError, ReceiptDetail, ResultCompatibility,
        ResultCompatibilityError, RunMode, RunRequest, Runtime, RuntimeConfig, RuntimeDecision,
        SnapshotLimits, TransactionStoreError, VfsError, VshError,
    };
    use crate::hook::{
        HookConfig, HookDecision, HookHandlerError, HookScope, HookVerdict, HookedRuntime,
        RequestEvent,
    };

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "vsh-runtime-{name}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).expect("unique test workspace should be created");
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

    #[test]
    fn auto_mode_commits_one_exact_virtual_result() {
        let directory = TestDirectory::new("auto");
        fs::write(directory.path().join("input.txt"), b"hello\n").unwrap();
        let runtime =
            Runtime::open(RuntimeConfig::new(directory.path()).with_in_process_execution())
                .unwrap();
        let receipt = runtime
            .run(
                RunRequest::new(
                    r"
from pathlib import Path
value = Path('/workspace/input.txt').read_text()
Path('/workspace/output.txt').write_text(value.upper())
len(value)
",
                )
                .with_mode(RunMode::Auto)
                .with_detail(ReceiptDetail::Full),
            )
            .unwrap();

        assert_eq!(receipt.state, TransactionState::Committed);
        assert!(matches!(receipt.decision, RuntimeDecision::AutoApproved));
        assert_eq!(receipt.changed_paths, 1);
        assert_eq!(receipt.changes.len(), 1);
        assert_eq!(
            fs::read(directory.path().join("output.txt")).unwrap(),
            b"HELLO\n"
        );
        assert!(receipt.commit.is_some());
    }

    #[test]
    fn oversized_program_is_rejected_before_workspace_snapshot() {
        let directory = TestDirectory::new("program-preflight");
        let runtime = Runtime::open(
            RuntimeConfig::new(directory.path())
                .with_snapshot_limits(SnapshotLimits {
                    max_nodes: 0,
                    ..SnapshotLimits::default()
                })
                .with_in_process_execution(),
        )
        .unwrap();
        let budget = ExecutionBudget {
            max_program_bytes: 1,
            ..ExecutionBudget::default()
        };

        let error = runtime
            .run(RunRequest::new("42").with_budget(budget))
            .unwrap_err();
        assert!(matches!(
            error,
            VshError::Execution(ExecutionError::Limit(source))
                if matches!(*source, vsh_monty::ExecutionLimitExceeded::ProgramBytes {
                    limit: 1,
                    attempted: 2,
                })
        ));
    }

    #[test]
    fn process_local_preview_cache_is_bounded_and_explicitly_releasable() {
        let directory = TestDirectory::new("ephemeral-capacity");
        let runtime = Runtime::open(
            RuntimeConfig::new(directory.path())
                .with_artifact_limits(ArtifactLimits {
                    max_ephemeral_entries: 1,
                    ..ArtifactLimits::default()
                })
                .with_in_process_execution(),
        )
        .unwrap();

        let first = runtime.preview(RunRequest::new("None")).unwrap();
        let error = runtime.preview(RunRequest::new("0")).unwrap_err();
        assert!(matches!(
            error,
            VshError::EphemeralCapacity {
                entries: 1,
                max_entries: 1,
                ..
            }
        ));

        assert!(runtime.discard_preview(first.transaction).unwrap());
        assert!(!runtime.discard_preview(first.transaction).unwrap());
        runtime.preview(RunRequest::new("1")).unwrap();
    }

    #[test]
    fn python_result_incompatibility_prevents_auto_commit() {
        let directory = TestDirectory::new("python-result");
        let runtime = Runtime::open(
            RuntimeConfig::new(directory.path())
                .with_result_compatibility(ResultCompatibility::Python)
                .with_in_process_execution(),
        )
        .unwrap();

        let error = runtime
            .run(
                RunRequest::new(
                    r"
from pathlib import Path
Path('/workspace/must-not-exist.txt').write_text('blocked')
type({}.keys())
",
                )
                .with_mode(RunMode::Auto),
            )
            .unwrap_err();

        assert!(matches!(error, VshError::ResultCompatibility(_)));
        assert!(!directory.path().join("must-not-exist.txt").exists());
    }

    #[test]
    fn strict_preview_requires_exact_approval_before_commit() {
        let directory = TestDirectory::new("approval");
        let runtime = Runtime::open(
            RuntimeConfig::new(directory.path())
                .with_policy_profile(PolicyProfile::Strict)
                .with_in_process_execution(),
        )
        .unwrap();
        let receipt = runtime
            .preview(RunRequest::new(
                "from pathlib import Path\nPath('/workspace/approved.txt').write_text('yes')",
            ))
            .unwrap();
        assert_eq!(receipt.state, TransactionState::PendingApproval);
        assert!(matches!(
            receipt.decision,
            RuntimeDecision::PendingApproval(_)
        ));
        assert!(!directory.path().join("approved.txt").exists());

        runtime
            .approve(
                receipt.transaction,
                PrincipalId::digest_label("independent-test-principal"),
                10,
                20,
            )
            .unwrap();
        let committed = runtime.commit(receipt.transaction, 11).unwrap();
        assert_eq!(committed.state, TransactionState::Committed);
        assert_eq!(
            fs::read(directory.path().join("approved.txt")).unwrap(),
            b"yes"
        );
    }

    #[test]
    fn approval_artifact_survives_runtime_restart() {
        let directory = TestDirectory::new("approval-restart");
        let config = RuntimeConfig::new(directory.path())
            .with_policy_profile(PolicyProfile::Strict)
            .with_in_process_execution();
        let receipt = Runtime::open(config.clone())
            .unwrap()
            .preview(
                RunRequest::new(
                    "from pathlib import Path\nPath('/workspace/restarted.txt').write_text('yes')\n{'answer': 42}",
                )
                .with_detail(ReceiptDetail::Full),
            )
            .unwrap();
        assert_eq!(receipt.state, TransactionState::PendingApproval);

        let reopened = Runtime::open(config).unwrap();
        reopened
            .approve(
                receipt.transaction,
                PrincipalId::digest_label("restart-principal"),
                100,
                200,
            )
            .unwrap();
        let committed = reopened.commit(receipt.transaction, 101).unwrap();

        assert_eq!(committed.state, TransactionState::Committed);
        assert_eq!(committed.value.py_repr(), "{'answer': 42}");
        assert_eq!(committed.changes, receipt.changes);
        assert_eq!(
            fs::read(directory.path().join("restarted.txt")).unwrap(),
            b"yes"
        );
    }

    #[test]
    fn caught_protected_read_deterministically_denies_all_changes() {
        let directory = TestDirectory::new("deny");
        fs::write(directory.path().join(".env"), b"TOKEN=secret\n").unwrap();
        let runtime =
            Runtime::open(RuntimeConfig::new(directory.path()).with_in_process_execution())
                .unwrap();
        let receipt = runtime
            .run(
                RunRequest::new(
                    r"
from pathlib import Path
try:
    Path('/workspace/.env').read_text()
except PermissionError:
    Path('/workspace/should-not-exist.txt').write_text('blocked')
",
                )
                .with_mode(RunMode::Auto),
            )
            .unwrap();

        assert_eq!(receipt.state, TransactionState::Denied);
        assert!(matches!(
            receipt.decision,
            RuntimeDecision::Denied(ref manifest)
                if matches!(manifest.reason, DenyReason::ProtectedAccessAttempt(_))
        ));
        assert!(!directory.path().join("should-not-exist.txt").exists());
    }

    #[test]
    fn stale_preview_never_overwrites_external_work() {
        let directory = TestDirectory::new("stale");
        fs::write(directory.path().join("input.txt"), b"before").unwrap();
        let runtime =
            Runtime::open(RuntimeConfig::new(directory.path()).with_in_process_execution())
                .unwrap();
        let receipt = runtime
            .preview(RunRequest::new(
                r"
from pathlib import Path
value = Path('/workspace/input.txt').read_text()
Path('/workspace/output.txt').write_text(value)
",
            ))
            .unwrap();
        fs::write(directory.path().join("input.txt"), b"external").unwrap();

        let error = runtime.commit(receipt.transaction, 0).unwrap_err();
        assert!(matches!(error, VshError::Commit(CommitError::Stale { .. })));
        assert!(!directory.path().join("output.txt").exists());
        assert_eq!(
            runtime.transaction(receipt.transaction).unwrap().state(),
            TransactionState::Stale
        );
    }

    #[test]
    fn explicit_data_directory_is_external_and_capability_rooted() {
        let workspace = TestDirectory::new("external-data-workspace");
        let data = TestDirectory::new("external-data-store");
        let config = RuntimeConfig::new(workspace.path())
            .with_data_directory(data.path())
            .with_in_process_execution();

        assert_eq!(config.workspace_root(), workspace.path());
        assert_eq!(config.data_directory(), data.path());
        assert!(config.worker_path().is_none());

        let runtime = Runtime::open(config).unwrap();
        runtime.preview(RunRequest::new("42")).unwrap();

        assert!(data.path().join("blobs").is_dir());
        assert!(data.path().join("transactions.lock").is_file());
        assert!(workspace.path().join(".vsh-runtime/transactions").is_dir());
    }

    #[test]
    fn explicit_data_directory_cannot_overlap_workspace() {
        let workspace = TestDirectory::new("overlapping-data");
        let data = workspace.path().join("caller-selected-data");
        let result = Runtime::open(
            RuntimeConfig::new(workspace.path())
                .with_data_directory(&data)
                .with_in_process_execution(),
        );

        assert!(matches!(result, Err(VshError::UnsafeDataDirectory { .. })));
        assert!(!data.exists());
    }

    #[cfg(unix)]
    #[test]
    fn default_runtime_symlink_fails_before_external_write() {
        use std::os::unix::fs::symlink;

        let workspace = TestDirectory::new("runtime-symlink-workspace");
        let outside = TestDirectory::new("runtime-symlink-outside");
        symlink(outside.path(), workspace.path().join(".vsh-runtime")).unwrap();

        let result =
            Runtime::open(RuntimeConfig::new(workspace.path()).with_in_process_execution());

        assert!(matches!(
            result,
            Err(VshError::Commit(CommitError::InternalIo { .. }))
        ));
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn canonical_alias_into_workspace_is_rejected_before_store_files() {
        use std::os::unix::fs::symlink;

        let workspace = TestDirectory::new("canonical-overlap-workspace");
        let alias_root = TestDirectory::new("canonical-overlap-alias");
        let alias = alias_root.path().join("workspace-alias");
        symlink(workspace.path(), &alias).unwrap();
        let data = alias.join("nested-data");

        let result = Runtime::open(
            RuntimeConfig::new(workspace.path())
                .with_data_directory(&data)
                .with_in_process_execution(),
        );

        assert!(matches!(result, Err(VshError::UnsafeDataDirectory { .. })));
        assert!(!workspace.path().join("nested-data").exists());
    }

    #[test]
    fn review_hook_receives_complete_canonical_evidence_and_can_commit() {
        let directory = TestDirectory::new("review-hook-approve");
        let observed = Arc::new(Mutex::new(None::<RequestEvent>));
        let observed_by_hook = Arc::clone(&observed);
        let runtime = HookedRuntime::open(
            RuntimeConfig::new(directory.path())
                .with_policy_profile(PolicyProfile::Strict)
                .with_in_process_execution(),
            HookConfig::new("security-review"),
            move |event: &RequestEvent| {
                *observed_by_hook.lock().unwrap() = Some(event.clone());
                Ok(HookDecision::approve("canonical evidence is safe"))
            },
        )
        .unwrap();

        let receipt = runtime
            .run(
                RunRequest::new(
                    "from pathlib import Path\nPath('/workspace/reviewed.txt').write_text('safe')",
                )
                .with_intent("create the reviewed output")
                .with_mode(RunMode::Auto),
                1_000,
            )
            .unwrap();

        assert_eq!(receipt.state, TransactionState::Committed);
        assert_eq!(
            fs::read(directory.path().join("reviewed.txt")).unwrap(),
            b"safe"
        );
        let event = observed.lock().unwrap().clone().unwrap();
        assert_eq!(event.transaction, receipt.transaction);
        assert_eq!(event.intent.as_deref(), Some("create the reviewed output"));
        assert_eq!(event.canonical_diff.len(), 1);
        assert_eq!(event.canonical_diff[0].path.as_str(), "reviewed.txt");
        assert!(!event.effects.is_empty());
        assert!(event.evidence_complete);
        assert!(!event.evidence_truncated);
    }

    #[test]
    fn review_content_binds_before_after_and_survives_restart_without_live_reads() {
        let directory = TestDirectory::new("review-content-restart");
        let path = directory.path().join("config.txt");
        fs::write(&path, b"before").unwrap();
        let config = RuntimeConfig::new(directory.path())
            .with_policy_profile(PolicyProfile::Strict)
            .with_commit_hook(HookConfig::new("content-review").with_max_content_bytes(1024))
            .with_in_process_execution();
        let runtime = Runtime::open(config.clone()).unwrap();
        let preview = runtime
            .preview(RunRequest::new(
                "vsh_write('/workspace/config.txt', 'after')",
            ))
            .unwrap();
        let prepared = runtime.prepare_commit(preview.transaction).unwrap();
        let event = prepared.event().unwrap();
        assert!(event.content_complete);
        assert_eq!(
            event
                .contents
                .iter()
                .map(|content| content.bytes.as_slice())
                .collect::<Vec<_>>(),
            vec![b"before".as_slice(), b"after".as_slice()]
        );
        assert!(
            event
                .contents
                .iter()
                .all(|content| content.path.as_str() == "config.txt")
        );
        drop(runtime);

        fs::write(&path, b"external change").unwrap();
        let restarted = Runtime::open(config).unwrap();
        let after_restart = restarted.prepare_commit(preview.transaction).unwrap();
        assert_eq!(after_restart.event(), prepared.event());
        assert!(matches!(
            restarted.resolve_commit(
                &after_restart,
                &HookDecision::approve("reviewed exact bytes"),
                1000
            ),
            Err(VshError::Commit(CommitError::Stale { .. }))
        ));
        assert_eq!(fs::read(path).unwrap(), b"external change");
    }

    #[test]
    fn review_content_budget_is_explicit_and_never_labels_partial_content_complete() {
        let directory = TestDirectory::new("review-content-budget");
        fs::write(directory.path().join("file.txt"), b"large before").unwrap();
        for maximum in [0, 3] {
            let runtime = Runtime::open(
                RuntimeConfig::new(directory.path())
                    .with_policy_profile(PolicyProfile::Strict)
                    .with_commit_hook(HookConfig::new("bounded").with_max_content_bytes(maximum))
                    .with_in_process_execution(),
            )
            .unwrap();
            let preview = runtime
                .preview(RunRequest::new("vsh_write('/workspace/file.txt', 'new')"))
                .unwrap();
            let prepared = runtime.prepare_commit(preview.transaction).unwrap();
            let event = prepared.event().unwrap();
            assert!(!event.content_complete);
            assert!(
                event
                    .contents
                    .iter()
                    .map(|content| content.bytes.len())
                    .sum::<usize>()
                    <= maximum
            );
            assert_eq!(
                fs::read(directory.path().join("file.txt")).unwrap(),
                b"large before"
            );
        }
    }

    #[test]
    fn read_only_review_contains_exact_observed_content_once() {
        let directory = TestDirectory::new("review-read-content");
        fs::write(directory.path().join("read.txt"), b"read evidence").unwrap();
        let runtime = Runtime::open(
            RuntimeConfig::new(directory.path())
                .with_commit_hook(
                    HookConfig::new("read-content")
                        .with_scope(HookScope::AllRequests)
                        .with_max_content_bytes(100),
                )
                .with_in_process_execution(),
        )
        .unwrap();
        let receipt = runtime
            .preview(RunRequest::new(
                "vsh_read('/workspace/read.txt')\nvsh_read('/workspace/read.txt')",
            ))
            .unwrap();
        let preparation = runtime.prepare_commit(receipt.transaction).unwrap();
        let event = preparation.event().unwrap();
        assert!(event.canonical_diff.is_empty());
        assert!(event.content_complete);
        assert_eq!(event.contents.len(), 1);
        assert_eq!(event.contents[0].bytes, b"read evidence");
    }

    #[test]
    fn empty_after_content_fits_an_exact_before_byte_budget() {
        let directory = TestDirectory::new("review-empty-content");
        fs::write(directory.path().join("file.txt"), b"abc").unwrap();
        let runtime = Runtime::open(
            RuntimeConfig::new(directory.path())
                .with_policy_profile(PolicyProfile::Strict)
                .with_commit_hook(HookConfig::new("empty-after").with_max_content_bytes(3))
                .with_in_process_execution(),
        )
        .unwrap();
        let preview = runtime
            .preview(RunRequest::new("vsh_write('/workspace/file.txt', '')"))
            .unwrap();
        let preparation = runtime.prepare_commit(preview.transaction).unwrap();
        let event = preparation.event().unwrap();
        assert!(event.content_complete);
        assert_eq!(event.contents.len(), 2);
        assert_eq!(event.contents[0].bytes, b"abc");
        assert!(event.contents[1].bytes.is_empty());
    }

    #[test]
    fn ready_preparation_cannot_bypass_an_applicable_hook() {
        let directory = TestDirectory::new("hook-forged-ready");
        let runtime = Runtime::open(
            RuntimeConfig::new(directory.path())
                .with_commit_hook(HookConfig::new("required").with_scope(HookScope::AllRequests))
                .with_in_process_execution(),
        )
        .unwrap();
        let receipt = runtime
            .preview(RunRequest::new(
                "vsh_write('/workspace/result.txt', 'must not commit')",
            ))
            .unwrap();
        runtime.prepare_commit(receipt.transaction).unwrap();
        let forged = super::CommitPreparation::Ready {
            transaction: receipt.transaction,
            state: receipt.state,
        };
        assert!(matches!(
            runtime.resolve_commit(&forged, &HookDecision::FollowPolicy, 1000),
            Err(VshError::HookRequired(_))
        ));
        assert!(!directory.path().join("result.txt").exists());
    }

    #[test]
    fn content_review_never_widens_native_read_permissions() {
        use vsh_policy::{AccessSet, CallPolicy, ProtectedRule, TransactionPolicy};

        let directory = TestDirectory::new("review-read-permissions");
        fs::write(directory.path().join("restricted.txt"), b"private before").unwrap();
        let policy = TransactionPolicy::new(
            PolicyProfile::Balanced,
            TransactionPolicy::default().thresholds(),
            CallPolicy::new(vec![
                ProtectedRule::new("restricted.txt", AccessSet::CONTENT_READ).unwrap(),
            ]),
        )
        .unwrap();
        let runtime = Runtime::open(
            RuntimeConfig::new(directory.path())
                .with_policy(policy)
                .with_commit_hook(
                    HookConfig::new("no-read")
                        .with_scope(HookScope::AllRequests)
                        .with_max_content_bytes(1024),
                )
                .with_in_process_execution(),
        )
        .unwrap();
        let receipt = runtime
            .preview(RunRequest::new(
                "vsh_write('/workspace/restricted.txt', 'replacement')",
            ))
            .unwrap();
        let prepared = runtime.prepare_commit(receipt.transaction).unwrap();
        let event = prepared.event().unwrap();
        assert!(!event.content_complete);
        assert!(event.contents.is_empty());
    }

    #[test]
    fn all_hook_can_return_feedback_and_keep_a_transaction_pending() {
        let directory = TestDirectory::new("hook-review-feedback");
        let calls = Arc::new(AtomicU64::new(0));
        let calls_by_hook = Arc::clone(&calls);
        let runtime = HookedRuntime::open(
            RuntimeConfig::new(directory.path()).with_in_process_execution(),
            HookConfig::new("evidence-judge").with_scope(HookScope::AllRequests),
            move |_event: &RequestEvent| {
                calls_by_hook.fetch_add(1, Ordering::Relaxed);
                Ok(HookDecision::review(
                    "generated file requires an explicit main-agent confirmation",
                ))
            },
        )
        .unwrap();
        let preview = runtime
            .preview(RunRequest::new(
                "from pathlib import Path\nPath('/workspace/check-me.txt').write_text('value')",
            ))
            .unwrap();
        assert_eq!(preview.state, TransactionState::AutoApproved);

        let resolution = runtime.commit(preview.transaction, 2_000).unwrap();
        assert_eq!(resolution.receipt.state, TransactionState::PendingApproval);
        let decision = resolution.hook.unwrap();
        assert_eq!(decision.verdict, HookVerdict::Review);
        assert!(decision.reason.contains("main-agent"));
        assert!(!directory.path().join("check-me.txt").exists());
        assert_eq!(calls.load(Ordering::Relaxed), 1);

        runtime
            .approve(
                preview.transaction,
                PrincipalId::digest_label("main-agent"),
                2_100,
                3_000,
            )
            .unwrap();
        let committed = runtime.commit(preview.transaction, 2_200).unwrap();
        assert_eq!(committed.receipt.state, TransactionState::Committed);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn all_requests_scope_delivers_read_only_evidence() {
        let directory = TestDirectory::new("hook-read-only");
        fs::write(directory.path().join("input.txt"), b"evidence").unwrap();
        let observed = Arc::new(Mutex::new(None::<RequestEvent>));
        let observed_by_hook = Arc::clone(&observed);
        let runtime = HookedRuntime::open(
            RuntimeConfig::new(directory.path()).with_in_process_execution(),
            HookConfig::new("read-review").with_scope(HookScope::AllRequests),
            move |event: &RequestEvent| {
                *observed_by_hook.lock().unwrap() = Some(event.clone());
                Ok(HookDecision::approve("bounded read is acceptable"))
            },
        )
        .unwrap();

        let receipt = runtime
            .run(
                RunRequest::new(
                    "from pathlib import Path\nPath('/workspace/input.txt').read_text()",
                )
                .with_mode(RunMode::Auto),
                10,
            )
            .unwrap();

        assert_eq!(receipt.state, TransactionState::Committed);
        assert_eq!(receipt.changed_paths, 0);
        let event = observed.lock().unwrap().clone().unwrap();
        assert!(event.canonical_diff.is_empty());
        assert!(
            event
                .effects
                .iter()
                .any(|effect| matches!(effect.effect, vsh_vfs::Effect::ContentRead { .. }))
        );
        assert!(event.execution.read_bytes > 0);
    }

    #[test]
    fn failed_hook_closes_auto_approval_into_pending_review() {
        let directory = TestDirectory::new("hook-failure");
        let runtime = HookedRuntime::open(
            RuntimeConfig::new(directory.path()).with_in_process_execution(),
            HookConfig::new("failing-hook").with_scope(HookScope::AllRequests),
            |_event: &RequestEvent| Err(HookHandlerError::new("judge unavailable")),
        )
        .unwrap();
        let preview = runtime
            .preview(RunRequest::new(
                "from pathlib import Path\nPath('/workspace/not-yet.txt').write_text('value')",
            ))
            .unwrap();

        let error = runtime.commit(preview.transaction, 0).unwrap_err();
        assert!(matches!(error, VshError::HookHandler(_)));
        assert_eq!(
            runtime.transaction(preview.transaction).unwrap().state(),
            TransactionState::PendingApproval
        );
        assert!(!directory.path().join("not-yet.txt").exists());
    }

    #[test]
    fn hard_policy_denial_never_invokes_hook() {
        let directory = TestDirectory::new("hook-hard-deny");
        fs::write(directory.path().join(".env"), b"secret").unwrap();
        let calls = Arc::new(AtomicU64::new(0));
        let calls_by_hook = Arc::clone(&calls);
        let runtime = HookedRuntime::open(
            RuntimeConfig::new(directory.path()).with_in_process_execution(),
            HookConfig::new("deny-proof").with_scope(HookScope::AllRequests),
            move |_event: &RequestEvent| {
                calls_by_hook.fetch_add(1, Ordering::Relaxed);
                Ok(HookDecision::approve("must not run"))
            },
        )
        .unwrap();
        let receipt = runtime
            .run(
                RunRequest::new(
                    "from pathlib import Path\ntry:\n    Path('/workspace/.env').read_text()\nexcept PermissionError:\n    pass",
                )
                .with_mode(RunMode::Auto),
                0,
            )
            .unwrap();

        assert_eq!(receipt.state, TransactionState::Denied);
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn native_error_surface_is_catchable_and_stable() {
        let directory = TestDirectory::new("runtime-errors");
        let not_a_directory = directory.path().join("file");
        fs::write(&not_a_directory, b"file").unwrap();
        let data_error = DataDirectory::open_trusted(&not_a_directory).unwrap_err();
        let transaction = vsh_types::TransactionId::from_bytes([7; 32]);
        let sourced = [
            VshError::DataDirectory(data_error),
            VshError::Blob(BlobStoreError::Io {
                operation: "read",
                path: PathBuf::from("blob"),
                source: std::io::Error::other("test"),
            }),
            VshError::Commit(CommitError::BaseSnapshotBinding),
            VshError::Execution(ExecutionError::UnsupportedSuspension {
                kind: "test",
                name: Some("name".to_owned()),
            }),
            VshError::Vfs(VfsError::RootMutation),
            VshError::Store(TransactionStoreError::NotFound { id: transaction }),
            VshError::Approval(ApprovalGrantError::InvalidWindow {
                issued_at_unix_ms: 2,
                expires_at_unix_ms: 1,
            }),
            VshError::CommitPlan(CommitPlanError::RootMutation),
            VshError::Artifact(ArtifactError::BindingMismatch),
            VshError::ResultCompatibility(ResultCompatibilityError::Depth {
                limit: 1,
                attempted: 2,
            }),
        ];
        for error in sourced {
            assert!(!error.to_string().is_empty());
            assert!(Error::source(&error).is_some());
        }

        let unsourced = [
            VshError::UnsafeDataDirectory {
                workspace_root: PathBuf::from("workspace"),
                data_directory: PathBuf::from("workspace/data"),
            },
            VshError::ArtifactBinding {
                requested: transaction,
                decoded: vsh_types::TransactionId::from_bytes([8; 32]),
            },
            VshError::RecoveryConflicts(Box::default()),
            VshError::MissingPending { transaction },
            VshError::DuplicatePending { transaction },
            VshError::EphemeralCapacity {
                entries: 2,
                max_entries: 1,
                attempted_bytes: 2,
                max_bytes: 1,
            },
            VshError::PendingPoisoned,
        ];
        for error in unsourced {
            assert!(!error.to_string().is_empty());
            assert!(Error::source(&error).is_none());
        }
    }
}
