use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
#[cfg(feature = "bash")]
use std::sync::Arc;
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

use vsh_commit::{
    CommitConfig, CommitError, CommitPlan, CommitPlanError, CommitReceipt, Committer,
    RecoveryReport, SnapshotLimits,
};
use vsh_monty::{
    ExecutionError, ExecutionLimits, ExecutionOutcome, ExecutionStats, InProcessConfig,
    InProcessMonty, ResultCompatibility, ResultCompatibilityError, SubprocessConfig,
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
    execution_evidence_digest, seal_pending_and_encode, seal_pending_and_size,
};
use crate::hook::{
    CommitPreparation, CommitResolution, HookBaseline, HookConfig, HookDecision,
    HookDecisionRecord, HookHandlerError, HookVerdict, RequestEvent,
};
use crate::output::{ExecutionOutput, Language};
#[cfg(feature = "bash")]
use crate::{BashConfig, BashResult};
use vsh_execution::ExecutionCancellation;

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
    /// Explicit guest language; existing requests default to Monty.
    pub language: Language,
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
            language: Language::Monty,
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

    /// Select an explicitly enabled guest language.
    #[must_use]
    pub const fn with_language(mut self, language: Language) -> Self {
        self.language = language;
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

impl From<PolicyDecision> for RuntimeDecision {
    fn from(decision: PolicyDecision) -> Self {
        match decision {
            PolicyDecision::Deny(manifest) => Self::Denied(manifest),
            PolicyDecision::AutoApprove => Self::AutoApproved,
            PolicyDecision::Escalate(manifest) => Self::PendingApproval(manifest),
        }
    }
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
    /// Complete backend-tagged result and byte-authoritative output.
    pub output: ExecutionOutput,
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
    #[cfg(feature = "bash")]
    bash: Option<BashConfig>,
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
            #[cfg(feature = "bash")]
            bash: None,
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

    /// Explicitly enable the separately isolated Bash worker.
    #[cfg(feature = "bash")]
    #[must_use]
    pub fn with_bash(mut self, config: BashConfig) -> Self {
        self.bash = Some(config);
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
    #[cfg(feature = "bash")]
    Deferred {
        config: Box<SubprocessConfig>,
        worker: Mutex<Option<Arc<SubprocessMonty>>>,
    },
    InProcess(Box<InProcessConfig>),
}

impl RuntimeExecution {
    fn open(config: &RuntimeConfig) -> Result<Self, ExecutionError> {
        let adapter = InProcessConfig::new(config.virtual_root.clone())
            .with_call_policy(config.policy.call_policy().clone());
        let Some(worker_path) = &config.worker_path else {
            return Ok(Self::InProcess(Box::new(adapter)));
        };
        let config_worker = SubprocessConfig::new(worker_path, adapter)
            .with_max_idle_workers(config.max_idle_workers);
        #[cfg(feature = "bash")]
        if config.bash.is_some() {
            return Ok(Self::Deferred {
                config: Box::new(config_worker),
                worker: Mutex::new(None),
            });
        }
        let worker = SubprocessMonty::new(config_worker)?;
        Ok(Self::Subprocess(Box::new(worker)))
    }

    fn adapter(&self) -> &InProcessConfig {
        match self {
            Self::Subprocess(worker) => worker.config().adapter(),
            #[cfg(feature = "bash")]
            Self::Deferred { config, .. } => config.adapter(),
            Self::InProcess(adapter) => adapter,
        }
    }

    fn security_digest(&self, adapter: &InProcessConfig) -> RuntimeConfigDigest {
        match self {
            Self::Subprocess(worker) => worker.config().security_digest_for(adapter),
            #[cfg(feature = "bash")]
            Self::Deferred { config, .. } => config.security_digest_for(adapter),
            Self::InProcess(_) => adapter.security_digest(),
        }
    }

    fn execute(
        &self,
        code: &str,
        filesystem: &mut VirtualFs,
        adapter: &InProcessConfig,
        cancellation: &ExecutionCancellation,
    ) -> Result<ExecutionOutcome, ExecutionError> {
        match self {
            Self::Subprocess(worker) => {
                worker.execute_cancellable(code, filesystem, adapter, cancellation)
            }
            #[cfg(feature = "bash")]
            Self::Deferred { config, worker } => {
                let mut slot = worker.lock().map_err(|_| ExecutionError::Cancelled)?;
                if slot.is_none() {
                    *slot = Some(Arc::new(SubprocessMonty::new_cancellable(
                        config.as_ref().clone(),
                        cancellation,
                    )?));
                }
                let worker = Arc::clone(slot.as_ref().expect("initialized worker"));
                drop(slot);
                worker.execute_cancellable(code, filesystem, adapter, cancellation)
            }
            Self::InProcess(_) => InProcessMonty::new(adapter.clone()).execute(code, filesystem),
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
    #[cfg(feature = "bash")]
    bash_execution: Mutex<Option<Arc<vsh_bash::SubprocessBash>>>,
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

struct GuestOutcome {
    output: ExecutionOutput,
    stats: ExecutionStats,
    denied_accesses: Vec<DeniedAccess>,
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
            #[cfg(feature = "bash")]
            bash_execution: Mutex::new(None),
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
        self.run_cancellable(request, &ExecutionCancellation::default())
    }

    /// Execute with request-scoped cancellation, atomically arbitrated against commit.
    ///
    /// # Errors
    /// Returns the same execution errors as [`Self::run`], or [`VshError::Cancelled`].
    pub fn run_cancellable(
        &self,
        request: RunRequest<'_>,
        cancellation: &ExecutionCancellation,
    ) -> Result<Receipt, VshError> {
        self.check_language(request.language)?;
        check_cancellation(cancellation)?;
        validate_program_size(request.code, request.budget)?;
        let total_started = Instant::now();
        let (mut filesystem, base_snapshot, base_node_count, snapshot_ns) =
            self.snapshot_filesystem()?;

        let execute_started = Instant::now();
        let (outcome, runtime_config) =
            self.execute_request(request, &mut filesystem, cancellation)?;
        check_cancellation(cancellation)?;
        let execute_ns = elapsed_ns(execute_started);

        let evaluated =
            self.evaluate_diff(&filesystem, &outcome.denied_accesses, base_node_count)?;
        let bind_started = Instant::now();
        let (mut binding, decision) = self.bind_candidate(
            request,
            &filesystem,
            base_snapshot,
            runtime_config,
            &evaluated,
        );
        let evidence = filesystem.into_evidence()?;
        let review = ReviewEvidence::capture(
            request.intent,
            evaluated.metrics,
            evidence.effects,
            self.config.artifact_limits,
        )?;
        self.seal_denied_evidence(&mut binding, &outcome, &review, &decision)?;
        let EvaluatedDiff {
            diff,
            diff_ns,
            policy_ns,
            ..
        } = evaluated;
        let transaction = binding.transaction_id();
        let (state, record) = Self::policy_record(transaction, base_snapshot, &decision)?;
        let mut receipt = Receipt {
            transaction,
            base_snapshot,
            state,
            decision,
            diff: diff.digest(),
            changed_paths: diff.entries().len(),
            changes: receipt_changes(request.detail, &diff),
            output: outcome.output,
            execution: outcome.stats,
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

        check_cancellation(cancellation)?;
        if state == TransactionState::Denied {
            check_cancellation(cancellation)?;
            self.store.create(record)?;
            receipt.timings.bind_and_store_ns = elapsed_ns(bind_started);
            receipt.timings.total_ns = elapsed_ns(total_started);
        } else {
            receipt = self.store_pending(
                PendingTransaction {
                    binding,
                    diff,
                    read_set: evidence.read_set,
                    write_set: evidence.write_set,
                    review,
                    receipt,
                },
                request.mode,
                bind_started,
                total_started,
            )?;
        }

        if cancellation.is_cancelled() {
            self.discard_cancelled(receipt.transaction)?;
            return Err(VshError::Cancelled);
        }
        if request.mode == RunMode::Auto && state == TransactionState::AutoApproved {
            receipt = match self.commit_cancellable(receipt.transaction, 0, cancellation) {
                Err(VshError::Cancelled) => {
                    self.discard_cancelled(receipt.transaction)?;
                    return Err(VshError::Cancelled);
                }
                result => result?,
            };
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

    /// Drop a cancelled call's ephemeral receipt and revoke durable automatic approval.
    ///
    /// Durable records remain auditable; explicit approval or entered commits are not
    /// rolled back. Call only for a receipt owned by the cancelled execution.
    ///
    /// # Errors
    /// Returns store or lock errors when fail-closed cleanup cannot be completed.
    pub fn discard_cancelled(&self, transaction: TransactionId) -> Result<(), VshError> {
        let removed = self.discard_preview(transaction)?;
        let record = match self.store.get(transaction) {
            Ok(record) => record,
            Err(TransactionStoreError::NotFound { id }) if removed && id == transaction => {
                return Ok(());
            }
            Err(source) => return Err(source.into()),
        };
        if record.state() == TransactionState::AutoApproved {
            self.store.compare_and_transition(
                transaction,
                TransactionState::AutoApproved,
                TransactionState::PendingApproval,
            )?;
        }
        Ok(())
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
        let artifact = self.load_pending(transaction)?;
        self.validate_policy(&artifact)?;
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
        self.commit_cancellable(transaction, now_unix_ms, &ExecutionCancellation::default())
    }

    /// Arbitrate cancellation before entering the commit/recovery protocol.
    ///
    /// # Errors
    /// Returns the errors of [`Self::commit`] or cancellation before commit entry.
    pub fn commit_cancellable(
        &self,
        transaction: TransactionId,
        now_unix_ms: u64,
        cancellation: &ExecutionCancellation,
    ) -> Result<Receipt, VshError> {
        check_cancellation(cancellation)?;
        if self.config.commit_hook.is_some() {
            let preparation = self.prepare_commit(transaction)?;
            if let Some(event) = preparation.event() {
                return Err(VshError::HookRequired(Box::new(event.clone())));
            }
        }
        if !cancellation.enter_commit() {
            return Err(VshError::Cancelled);
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
        self.validate_policy(&artifact)?;
        self.validate_output(&artifact.receipt.output)?;
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
        self.resolve_commit_cancellable(
            preparation,
            decision,
            now_unix_ms,
            &ExecutionCancellation::default(),
        )
    }

    /// Resolve one hook decision with request-scoped cancellation arbitration.
    ///
    /// # Errors
    /// Returns the errors of [`Self::resolve_commit`] or pre-commit cancellation.
    pub fn resolve_commit_cancellable(
        &self,
        preparation: &CommitPreparation,
        decision: &HookDecision,
        now_unix_ms: u64,
        cancellation: &ExecutionCancellation,
    ) -> Result<CommitResolution, VshError> {
        check_cancellation(cancellation)?;
        let transaction = preparation.transaction();
        let prepared_state = preparation.prepared_state();
        let artifact = self.load_pending(transaction)?;
        self.validate_policy(&artifact)?;
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
        let committing = matches!(decision, HookDecision::Approve { .. })
            || (*decision == HookDecision::FollowPolicy
                && matches!(
                    prepared_state,
                    TransactionState::AutoApproved | TransactionState::Approved
                ));
        if committing && !cancellation.enter_commit() {
            return Err(VshError::Cancelled);
        }
        check_cancellation(cancellation)?;
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
        self.validate_policy(&artifact)?;
        self.validate_output(&artifact.receipt.output)?;
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
        self.validate_policy(artifact)?;
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
            schema_version: 2,
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
            execution_context: crate::hook::ExecutionContext {
                language: artifact.receipt.output.language(),
                profile: match &artifact.receipt.output {
                    ExecutionOutput::Bash(result) => Some(result.profile.clone()),
                    ExecutionOutput::Monty { .. } => None,
                },
                exit_code: match &artifact.receipt.output {
                    ExecutionOutput::Bash(result) => Some(result.exit_code),
                    ExecutionOutput::Monty { .. } => None,
                },
                complete: artifact.review.complete
                    && !artifact.review.truncated
                    && artifact.binding.execution_evidence.is_some(),
                evidence: artifact.binding.execution_evidence,
                stdout_bytes: artifact.receipt.output.stdout_bytes().len(),
                stderr_bytes: artifact.receipt.output.stderr_bytes().len(),
            },
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
                (entry.kind != vsh_types::DiffKind::MetadataChange
                    && entry.before.is_some()
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
        self.execution.adapter().clone().with_limits(budget)
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
        // This is the fresh-seal insertion path; the creator already calculated
        // and installed the final, evidence-bound receipt identity.
        let transaction = artifact.receipt.transaction;
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
        mut artifact: PendingTransaction,
        bind_started: Instant,
        total_started: Instant,
    ) -> Result<Receipt, VshError> {
        let encoded = seal_pending_and_encode(&mut artifact, self.config.artifact_limits)?;
        let (_, record) = Self::policy_record(
            artifact.receipt.transaction,
            artifact.binding.base_snapshot,
            &artifact.receipt.decision,
        )?;
        let artifact_id = self.artifacts.put(&encoded)?;
        self.store.create(record.with_artifact(artifact_id))?;
        artifact.receipt.timings.bind_and_store_ns = elapsed_ns(bind_started);
        artifact.receipt.timings.total_ns = elapsed_ns(total_started);
        Ok(artifact.receipt)
    }

    fn store_pending(
        &self,
        artifact: PendingTransaction,
        mode: RunMode,
        bind_started: Instant,
        total_started: Instant,
    ) -> Result<Receipt, VshError> {
        if mode == RunMode::Preview && artifact.receipt.state == TransactionState::AutoApproved {
            self.retain_ephemeral(artifact, bind_started, total_started)
        } else {
            self.persist_pending(artifact, bind_started, total_started)
        }
    }

    fn retain_ephemeral(
        &self,
        mut artifact: PendingTransaction,
        bind_started: Instant,
        total_started: Instant,
    ) -> Result<Receipt, VshError> {
        let encoded_bytes = seal_pending_and_size(&mut artifact, self.config.artifact_limits)?;
        artifact.receipt.timings.bind_and_store_ns = elapsed_ns(bind_started);
        artifact.receipt.timings.total_ns = elapsed_ns(total_started);
        let receipt = artifact.receipt.clone();
        self.insert_pending(artifact, encoded_bytes)?;
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

    fn execute_request(
        &self,
        request: RunRequest<'_>,
        filesystem: &mut VirtualFs,
        cancellation: &ExecutionCancellation,
    ) -> Result<(GuestOutcome, RuntimeConfigDigest), VshError> {
        #[cfg(feature = "bash")]
        if request.language == Language::Bash {
            let config = self
                .config
                .bash
                .as_ref()
                .ok_or(VshError::LanguageUnavailable {
                    language: Language::Bash,
                })?;
            let mut slot = self
                .bash_execution
                .lock()
                .map_err(|_| VshError::PendingPoisoned)?;
            if slot.is_none() {
                *slot = Some(Arc::new(
                    vsh_bash::SubprocessBash::new_cancellable(config.clone(), cancellation)
                        .map_err(|source| VshError::Bash {
                            source,
                            changes: Vec::new(),
                            changes_complete: true,
                        })?,
                ));
            }
            let worker = Arc::clone(slot.as_ref().expect("initialized Bash worker"));
            drop(slot);
            let result = worker.execute_cancellable(
                request.code,
                filesystem,
                self.config.policy.call_policy(),
                request.budget,
                cancellation,
            );
            let outcome = result.map_err(|source| {
                let diagnostic = filesystem.canonical_diff();
                VshError::Bash {
                    source,
                    changes_complete: diagnostic.is_ok(),
                    changes: diagnostic.map_or_else(
                        |_| Vec::new(),
                        |diff| receipt_changes(ReceiptDetail::Full, &diff),
                    ),
                }
            })?;
            let digest = aggregate_runtime_digest(
                config.security_digest(request.budget),
                self.config.snapshot_limits,
                self.config.commit_config,
                self.config.store_config,
                self.config.artifact_limits,
                self.config.result_compatibility,
                self.config.commit_hook,
            );
            return Ok((
                GuestOutcome {
                    output: ExecutionOutput::Bash(BashResult {
                        profile: vsh_bash::PROFILE_ID.to_owned(),
                        exit_code: outcome.exit_code,
                        stdout: outcome.stdout,
                        stderr: outcome.stderr,
                    }),
                    stats: outcome.stats,
                    denied_accesses: outcome.denied_accesses,
                },
                digest,
            ));
        }
        let config = self.monty_config(request.budget);
        let digest = self.runtime_config_digest(&config);
        let outcome = self
            .execution
            .execute(request.code, filesystem, &config, cancellation)?;
        validate_result_compatibility(&outcome.value, self.config.result_compatibility)?;
        Ok((
            GuestOutcome {
                output: ExecutionOutput::Monty {
                    value: outcome.value,
                    stdout: outcome.stdout,
                },
                stats: outcome.stats,
                denied_accesses: outcome.denied_accesses,
            },
            digest,
        ))
    }

    fn check_language(&self, language: Language) -> Result<(), VshError> {
        if language == Language::Monty {
            return Ok(());
        }
        #[cfg(feature = "bash")]
        if self.config.bash.is_some() {
            if self.config.virtual_root.as_str() != "/workspace" {
                return Err(VshError::Bash {
                    source: vsh_bash::BashError::Configuration(
                        "Bash requires the /workspace virtual root".into(),
                    ),
                    changes: Vec::new(),
                    changes_complete: true,
                });
            }
            return Ok(());
        }
        Err(VshError::LanguageUnavailable { language })
    }

    fn validate_policy(&self, artifact: &PendingTransaction) -> Result<(), VshError> {
        if artifact.binding.policy != self.config.policy.digest() {
            return Err(VshError::PolicyChanged {
                transaction: artifact.binding.transaction_id(),
            });
        }
        Ok(())
    }

    fn validate_output(&self, output: &ExecutionOutput) -> Result<(), VshError> {
        if let Some(value) = output.monty_value() {
            validate_result_compatibility(value, self.config.result_compatibility)?;
        }
        Ok(())
    }

    fn bind_candidate(
        &self,
        request: RunRequest<'_>,
        filesystem: &VirtualFs,
        base_snapshot: SnapshotId,
        runtime_config: RuntimeConfigDigest,
        evaluated: &EvaluatedDiff,
    ) -> (vsh_types::TransactionBinding, RuntimeDecision) {
        let decision = RuntimeDecision::from(evaluated.decision.clone());
        let binding = bind_transaction(TransactionIdentityInput {
            base_snapshot,
            diff: &evaluated.diff,
            read_set: filesystem.read_set(),
            write_set: filesystem.write_set(),
            program: request.code,
            policy: &self.config.policy,
            runtime_config,
            intent: request.intent,
        });
        (binding, decision)
    }

    fn seal_denied_evidence(
        &self,
        binding: &mut vsh_types::TransactionBinding,
        outcome: &GuestOutcome,
        review: &ReviewEvidence,
        decision: &RuntimeDecision,
    ) -> Result<(), VshError> {
        // Actionable artifacts are sealed in their single encode/count pass.
        // A denied receipt has no pending artifact to encode.
        if matches!(decision, RuntimeDecision::Denied(_)) {
            binding.execution_evidence = Some(execution_evidence_digest(
                &outcome.output,
                outcome.stats,
                review,
                decision,
                self.config.artifact_limits,
            )?);
        }
        Ok(())
    }

    fn policy_record(
        transaction: TransactionId,
        base_snapshot: SnapshotId,
        decision: &RuntimeDecision,
    ) -> Result<(TransactionState, TransactionRecord), VshError> {
        let mut record = TransactionRecord::new(transaction, base_snapshot);
        record
            .transition(TransactionState::Running)
            .map_err(TransactionStoreError::Transition)?;
        record
            .transition(TransactionState::VirtualComplete)
            .map_err(TransactionStoreError::Transition)?;
        let state = match decision {
            RuntimeDecision::Denied(_) => {
                record
                    .transition(TransactionState::Denied)
                    .map_err(TransactionStoreError::Transition)?;
                TransactionState::Denied
            }
            RuntimeDecision::AutoApproved => {
                record
                    .transition(TransactionState::AutoApproved)
                    .map_err(TransactionStoreError::Transition)?;
                TransactionState::AutoApproved
            }
            RuntimeDecision::PendingApproval(_) => {
                record
                    .transition(TransactionState::PendingApproval)
                    .map_err(TransactionStoreError::Transition)?;
                TransactionState::PendingApproval
            }
        };
        Ok((state, record))
    }
}

fn check_cancellation(cancellation: &ExecutionCancellation) -> Result<(), VshError> {
    if cancellation.is_cancelled() {
        Err(VshError::Cancelled)
    } else {
        Ok(())
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
    /// Cancellation won before the host commit boundary.
    Cancelled,
    /// The requested frontend was not enabled by the host.
    LanguageUnavailable {
        /// Frontend requested without host opt-in.
        language: Language,
    },
    /// Noncommittable Bash diagnostics; there is deliberately no transaction ID.
    #[cfg(feature = "bash")]
    Bash {
        /// Typed terminal execution failure.
        source: vsh_bash::BashError,
        /// Noncommittable virtual change diagnostics.
        changes: Vec<DiffEntry>,
        /// Whether a canonical diagnostic diff could be computed; never permission to commit.
        changes_complete: bool,
    },
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
    /// The pending transaction was evaluated under a different policy identity.
    PolicyChanged {
        /// Transaction that must be previewed again under the active policy.
        transaction: TransactionId,
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
    #[expect(
        clippy::too_many_lines,
        reason = "one exhaustive typed-error projection preserves the SDK error contract"
    )]
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("VSH request cancelled before commit"),
            Self::LanguageUnavailable { language } => write!(
                formatter,
                "{language:?} execution is not enabled by the host"
            ),
            #[cfg(feature = "bash")]
            Self::Bash { source, .. } => fmt::Display::fmt(source, formatter),
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
            Self::PolicyChanged { transaction } => write!(
                formatter,
                "policy changed for transaction {transaction}; create a fresh preview"
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
            Self::Cancelled | Self::LanguageUnavailable { .. } => None,
            #[cfg(feature = "bash")]
            Self::Bash { source, .. } => Some(source),
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
            | Self::PolicyChanged { .. }
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
        DataDirectory, ExecutionBudget, ExecutionError, InProcessConfig, ReceiptDetail,
        ResultCompatibility, ResultCompatibilityError, RunMode, RunRequest, Runtime, RuntimeConfig,
        RuntimeDecision, SnapshotLimits, TransactionStoreError, VfsError, VirtualRoot, VshError,
    };
    use crate::hook::{
        HookConfig, HookDecision, HookHandlerError, HookScope, HookVerdict, HookedRuntime,
        RequestEvent,
    };

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[cfg(all(unix, feature = "bash"))]
    fn bash_config() -> crate::BashConfig {
        let executable = std::env::var_os("VSH_BASH_WORKER").map_or_else(
            || {
                std::env::current_exe()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .parent()
                    .unwrap()
                    .join("vsh-bash-worker")
            },
            PathBuf::from,
        );
        assert!(
            executable.is_file(),
            "build vsh-bash-worker before public runtime tests"
        );
        crate::BashConfig::new(executable)
    }

    #[cfg(all(unix, feature = "bash"))]
    #[test]
    fn bash_preview_restart_hook_and_commit_use_one_canonical_pipeline() {
        let workspace = TestDirectory::new("bash-public");
        fs::write(workspace.path().join("input.txt"), "old\n").unwrap();
        fs::write(workspace.path().join("binary.bin"), [0xff]).unwrap();
        let config = RuntimeConfig::new(workspace.path())
            .with_worker_path("/missing-monty-worker")
            .with_bash(bash_config())
            .with_commit_hook(HookConfig::new("bash-test").with_scope(HookScope::AllRequests));
        let runtime = Runtime::open(config.clone()).unwrap();
        let receipt = runtime
            .preview(
                RunRequest::new(
                    "cat input.txt | sed 's/old/new/' > output.txt; cat binary.bin; printf err >&2",
                )
                .with_language(crate::Language::Bash)
                .with_detail(ReceiptDetail::Full),
            )
            .unwrap();
        assert_eq!(receipt.state, TransactionState::AutoApproved);
        assert!(!workspace.path().join("output.txt").exists());
        let crate::ExecutionOutput::Bash(result) = &receipt.output else {
            panic!("expected Bash output")
        };
        assert_eq!(result.stdout, [0xff]);
        assert_eq!(result.stderr, b"err");
        let preparation = runtime.prepare_commit(receipt.transaction).unwrap();
        let event = preparation.event().unwrap();
        assert_eq!(event.execution_context.language, crate::Language::Bash);
        assert!(event.execution_context.complete);
        assert!(
            event
                .effects
                .iter()
                .any(|effect| effect.origin == crate::EffectOrigin::BashCall)
        );
        drop(runtime);
        let reopened = Runtime::open(config).unwrap();
        let regenerated = reopened.prepare_commit(receipt.transaction).unwrap();
        assert_eq!(regenerated.event(), preparation.event());
        let committed = reopened
            .resolve_commit(
                &regenerated,
                &HookDecision::approve("checked actual diff"),
                1,
            )
            .unwrap()
            .receipt;
        assert_eq!(committed.output, receipt.output);
        assert_eq!(committed.state, TransactionState::Committed);
        assert_eq!(
            fs::read(workspace.path().join("output.txt")).unwrap(),
            b"new\n"
        );
        assert!(reopened.commit(receipt.transaction, 2).is_err());
    }

    #[cfg(all(unix, feature = "bash"))]
    #[test]
    fn bash_auto_stale_nonzero_denial_and_disabled_paths_fail_closed() {
        let workspace = TestDirectory::new("bash-failures");
        fs::write(workspace.path().join("input.txt"), "initial").unwrap();
        fs::write(workspace.path().join(".env"), "mock-secret").unwrap();
        let config = RuntimeConfig::new(workspace.path())
            .with_in_process_execution()
            .with_bash(bash_config());
        let runtime = Runtime::open(config).unwrap();
        let receipt = runtime
            .run(
                RunRequest::new("printf success > created.txt")
                    .with_language(crate::Language::Bash)
                    .with_mode(RunMode::Auto),
            )
            .unwrap();
        assert_eq!(receipt.state, TransactionState::Committed);
        assert_eq!(
            fs::read(workspace.path().join("created.txt")).unwrap(),
            b"success"
        );
        let stale = runtime
            .preview(
                RunRequest::new("cat input.txt > stale.txt").with_language(crate::Language::Bash),
            )
            .unwrap();
        fs::write(workspace.path().join("input.txt"), "external").unwrap();
        assert!(matches!(
            runtime.commit(stale.transaction, 0),
            Err(VshError::Commit(CommitError::Stale { .. }))
        ));
        assert!(!workspace.path().join("stale.txt").exists());
        let failed = runtime
            .run(
                RunRequest::new("printf partial > partial.txt; cat .env; exit 7")
                    .with_language(crate::Language::Bash)
                    .with_mode(RunMode::Auto),
            )
            .unwrap_err();
        let VshError::Bash {
            source:
                crate::BashError::Exit {
                    code,
                    denied_accesses,
                    ..
                },
            changes,
            changes_complete,
        } = failed
        else {
            panic!("expected typed nonzero failure")
        };
        assert_eq!(code, 7);
        assert_eq!(denied_accesses.len(), 1);
        assert_eq!(changes.len(), 1);
        assert!(changes_complete);
        assert!(!workspace.path().join("partial.txt").exists());
        let denied = runtime
            .run(
                RunRequest::new("cat .env || true; printf denied > denied.txt")
                    .with_language(crate::Language::Bash)
                    .with_mode(RunMode::Auto),
            )
            .unwrap();
        assert_eq!(denied.state, TransactionState::Denied);
        assert!(!workspace.path().join("denied.txt").exists());
        let disabled =
            Runtime::open(RuntimeConfig::new(workspace.path()).with_in_process_execution())
                .unwrap();
        assert!(matches!(
            disabled.preview(RunRequest::new("true").with_language(crate::Language::Bash)),
            Err(VshError::LanguageUnavailable { .. })
        ));
    }

    #[test]
    fn cancellation_before_execution_or_commit_cannot_mutate_host() {
        let workspace = TestDirectory::new("cancel-public");
        let runtime =
            Runtime::open(RuntimeConfig::new(workspace.path()).with_in_process_execution())
                .unwrap();
        let token = crate::ExecutionCancellation::default();
        assert!(token.cancel());
        assert!(matches!(
            runtime.run_cancellable(
                RunRequest::new("vsh_write('cancelled.txt', 'no')").with_mode(RunMode::Auto),
                &token
            ),
            Err(VshError::Cancelled)
        ));
        let receipt = runtime
            .preview(RunRequest::new("vsh_write('cancelled.txt', 'no')"))
            .unwrap();
        assert!(matches!(
            runtime.commit_cancellable(receipt.transaction, 0, &token),
            Err(VshError::Cancelled)
        ));
        assert!(!workspace.path().join("cancelled.txt").exists());
    }

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
    fn cached_monty_adapter_preserves_policy_namespace_and_request_limits() {
        use vsh_policy::{AccessSet, CallPolicy, ProtectedRule, TransactionPolicy};

        let directory = TestDirectory::new("monty-adapter-template");
        let policy = TransactionPolicy::new(
            PolicyProfile::Strict,
            TransactionPolicy::default().thresholds(),
            CallPolicy::new(vec![
                ProtectedRule::new("private.txt", AccessSet::CONTENT_READ).unwrap(),
            ]),
        )
        .unwrap();
        let config = RuntimeConfig::new(directory.path())
            .with_in_process_execution()
            .with_virtual_root(VirtualRoot::new("/project").unwrap())
            .with_policy(policy);
        let budget = ExecutionBudget {
            max_os_calls: 17,
            max_evidence_records: 53,
            ..ExecutionBudget::default()
        };
        let expected = InProcessConfig::new(config.virtual_root.clone())
            .with_call_policy(config.policy.call_policy().clone())
            .with_limits(budget);
        let runtime = Runtime::open(config.clone()).unwrap();
        assert_eq!(runtime.monty_config(budget), expected);
        assert_eq!(
            runtime.execution.adapter().limits(),
            ExecutionBudget::default()
        );
        assert_ne!(
            runtime.runtime_config_digest(&expected),
            runtime.runtime_config_digest(&runtime.monty_config(ExecutionBudget::default()))
        );

        #[cfg(feature = "bash")]
        {
            let deferred = Runtime::open(
                config
                    .with_worker_path("/missing-monty-worker")
                    .with_bash(crate::BashConfig::new("/missing-bash-worker")),
            )
            .unwrap();
            assert_eq!(deferred.monty_config(budget), expected);
            assert_eq!(
                deferred.execution.adapter().limits(),
                ExecutionBudget::default()
            );
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
    fn changed_policy_blocks_every_pending_authority_path_after_restart() {
        for (profile, hook, approved) in [
            (PolicyProfile::Balanced, false, false),
            (PolicyProfile::Strict, false, false),
            (PolicyProfile::Strict, false, true),
            (PolicyProfile::Balanced, true, false),
            (PolicyProfile::Strict, true, false),
        ] {
            let directory = TestDirectory::new("changed-policy");
            let mut config = RuntimeConfig::new(directory.path())
                .with_policy_profile(profile)
                .with_in_process_execution();
            if hook {
                config = config.with_commit_hook(
                    HookConfig::new("policy-check").with_scope(HookScope::AllRequests),
                );
            }
            let runtime = Runtime::open(config.clone()).unwrap();
            let receipt = runtime
                .preview(RunRequest::new(
                    "vsh_write('/workspace/output.txt', 'blocked')",
                ))
                .unwrap();
            if approved {
                runtime
                    .approve(
                        receipt.transaction,
                        PrincipalId::digest_label("reviewer"),
                        1,
                        100,
                    )
                    .unwrap();
            }
            // Also makes an auto-approved, process-local preview durable.
            let prepared = runtime.prepare_commit(receipt.transaction).unwrap();
            let before = runtime.transaction(receipt.transaction).unwrap();
            drop(runtime);

            let changed =
                Runtime::open(config.clone().with_policy_profile(PolicyProfile::Paranoid)).unwrap();
            for error in [
                changed
                    .approve(
                        receipt.transaction,
                        PrincipalId::digest_label("reviewer"),
                        1,
                        100,
                    )
                    .unwrap_err(),
                changed.prepare_commit(receipt.transaction).unwrap_err(),
                changed
                    .resolve_commit(&prepared, &HookDecision::FollowPolicy, 2)
                    .unwrap_err(),
                changed.commit(receipt.transaction, 2).unwrap_err(),
            ] {
                assert!(
                    matches!(error, VshError::PolicyChanged { transaction } if transaction == receipt.transaction)
                );
            }
            assert_eq!(changed.transaction(receipt.transaction).unwrap(), before);
            assert!(!directory.path().join("output.txt").exists());
            // Inspection and journal recovery remain available after a policy change.
            changed.load_pending(receipt.transaction).unwrap();
            changed.recover().unwrap();
            drop(changed);

            let original = Runtime::open(config).unwrap();
            let prepared = original.prepare_commit(receipt.transaction).unwrap();
            if hook {
                original
                    .resolve_commit(&prepared, &HookDecision::approve("exact evidence"), 2)
                    .unwrap();
            } else {
                if !approved && profile == PolicyProfile::Strict {
                    original
                        .approve(
                            receipt.transaction,
                            PrincipalId::digest_label("reviewer"),
                            1,
                            100,
                        )
                        .unwrap();
                }
                original.commit(receipt.transaction, 2).unwrap();
            }
            assert_eq!(
                fs::read(directory.path().join("output.txt")).unwrap(),
                b"blocked"
            );
        }
    }

    #[test]
    fn changed_policy_does_not_block_recovery_of_an_entered_commit() {
        use vsh_commit::FaultPoint;
        use vsh_store::TransactionStore;

        for point in [
            FaultPoint::OperationApplied(0),
            FaultPoint::CommitMarkerSynced,
        ] {
            let directory = TestDirectory::new("policy-recovery");
            fs::write(directory.path().join("file.txt"), b"before").unwrap();
            let config = RuntimeConfig::new(directory.path()).with_in_process_execution();
            let runtime = Runtime::open(config.clone()).unwrap();
            let receipt = runtime
                .preview(RunRequest::new("vsh_write('/workspace/file.txt', 'after')"))
                .unwrap();
            runtime.prepare_commit(receipt.transaction).unwrap();
            let artifact = runtime.load_pending(receipt.transaction).unwrap();
            let plan = super::CommitPlan::new(
                &artifact.binding,
                &artifact.diff,
                &artifact.read_set,
                &artifact.write_set,
            )
            .unwrap();
            let reservation = runtime.store.reserve(receipt.transaction, 0).unwrap();
            assert!(
                runtime
                    .committer
                    .commit_with_faults(&runtime.store, reservation, &plan, &|candidate| candidate
                        == point)
                    .is_err()
            );
            drop(runtime);

            let reopened =
                Runtime::open(config.with_policy_profile(PolicyProfile::Paranoid)).unwrap();
            let (state, bytes): (_, &[u8]) = if point == FaultPoint::CommitMarkerSynced {
                (TransactionState::Committed, b"after")
            } else {
                (TransactionState::Failed, b"before")
            };
            assert_eq!(
                reopened.transaction(receipt.transaction).unwrap().state(),
                state
            );
            assert_eq!(fs::read(directory.path().join("file.txt")).unwrap(), bytes);
            assert!(reopened.recover().unwrap().conflicts.is_empty());
        }
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
        let persisted = reopened.load_pending(receipt.transaction).unwrap();
        assert!(persisted.binding.execution_evidence.is_some());
        assert!(persisted.review.complete);
        assert_eq!(persisted.receipt.output, receipt.output);
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
        assert_eq!(
            committed.output.monty_value().unwrap().py_repr(),
            "{'answer': 42}"
        );
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
            VshError::PolicyChanged { transaction },
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
