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
    DenyManifest, PolicyDecision, PolicyInput, PolicyProfile, RiskManifest,
    TransactionIdentityInput, TransactionPolicy, bind_transaction,
};
use vsh_store::{
    ApprovalGrant, ApprovalGrantError, BlobStore, BlobStoreError, DataDirectory,
    DataDirectoryError, FileStoreConfig, FileTransactionStore, TransactionRecord, TransactionStore,
    TransactionStoreError,
};
use vsh_types::{
    DiffDigest, DiffEntry, RuntimeConfigDigest, SnapshotId, TransactionId, TransactionState,
};
use vsh_vfs::{VfsError, VirtualFs};

use crate::artifact::{ArtifactError, PendingTransaction, decode_pending, encode_pending};

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
        let runtime_config = aggregate_runtime_digest(
            self.execution.security_digest(&monty_config),
            self.config.snapshot_limits,
            self.config.commit_config,
            self.config.store_config,
            self.config.artifact_limits,
            self.config.result_compatibility,
        );
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

        let diff_started = Instant::now();
        let diff = filesystem.canonical_diff()?;
        let diff_ns = elapsed_ns(diff_started);

        let policy_started = Instant::now();
        let policy_decision = self.config.policy.evaluate(PolicyInput {
            diff: &diff,
            effects: filesystem.effects(),
            denied_accesses: &denied_accesses,
            base_node_count,
        });
        let policy_ns = elapsed_ns(policy_started);

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

    fn monty_config(&self, budget: ExecutionBudget) -> InProcessConfig {
        InProcessConfig::new(self.config.virtual_root.clone())
            .with_limits(budget)
            .with_call_policy(self.config.policy.call_policy().clone())
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
) -> RuntimeConfigDigest {
    let mut canonical = Vec::with_capacity(33 + 8 * 19);
    canonical.extend_from_slice(b"vsh-runtime-config-v4");
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
    encode_usize(artifact.max_ephemeral_entries, &mut canonical);
    encode_usize(artifact.max_ephemeral_bytes, &mut canonical);
    canonical.push(match result_compatibility {
        ResultCompatibility::Native => 0,
        ResultCompatibility::Python => 1,
    });
    RuntimeConfigDigest::digest_canonical(&canonical)
}

fn encode_usize(value: usize, output: &mut Vec<u8>) {
    output.extend_from_slice(&u64::try_from(value).unwrap_or(u64::MAX).to_le_bytes());
}

fn receipt_changes(detail: ReceiptDetail, diff: &vsh_vfs::CanonicalDiff) -> Vec<DiffEntry> {
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
            Self::RecoveryConflicts(_)
            | Self::UnsafeDataDirectory { .. }
            | Self::ArtifactBinding { .. }
            | Self::MissingPending { .. }
            | Self::DuplicatePending { .. }
            | Self::EphemeralCapacity { .. }
            | Self::PendingPoisoned => None,
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

    use vsh_commit::CommitError;
    use vsh_policy::{DenyReason, PolicyProfile};
    use vsh_types::{PrincipalId, TransactionState};

    use super::{
        ApprovalGrantError, ArtifactError, ArtifactLimits, BlobStoreError, CommitPlanError,
        DataDirectory, ExecutionBudget, ExecutionError, ReceiptDetail, ResultCompatibility,
        ResultCompatibilityError, RunMode, RunRequest, Runtime, RuntimeConfig, RuntimeDecision,
        SnapshotLimits, TransactionStoreError, VfsError, VshError,
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
