use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cap_std::ambient_authority;
use cap_std::fs::Dir;
use vsh_store::{
    BlobStore, BlobStoreError, CommitReservation, DataDirectory, DataDirectoryError,
    TransactionStore, TransactionStoreError,
};
use vsh_types::{
    BlobId, ContentVersion, DirectoryDigest, NodeKind, NodeState, PlatformFileId, TransactionId,
    TransactionState, VPath,
};
use vsh_vfs::{BaseSnapshot, ReadObservation};

use crate::host::{
    HostError, SnapshotLimits, capture_snapshot, content_digest, create_new_file,
    create_staged_symlink, directory_digest, open_coordination_file, open_or_create_real_dir,
    open_real_dir, open_real_file, relative_path, relocated_state_matches, set_dir_mode,
    set_file_mode, stamp_at, stamp_dir, stamp_file, state_matches, sync_dir, sync_installed_file,
    validate_symlink_target, witness_matches,
};
use crate::journal::{
    JOURNAL_FILE, Journal, JournalError, JournalState, PLAN_FILE, QUARANTINE_DIRECTORY,
    STAGE_DIRECTORY, Witness, has_valid_commit_marker, read_journal, write_commit_marker,
};
use crate::plan::{
    CommitPlan, CommitPlanError, Operation, PlanDecodeError, PreparedPlan, quarantine_name,
    stage_link_name, stage_name,
};

const DIRECTORY_OWNER_MARKER: &str = ".vsh-runtime-owner";
const DIRECTORY_OWNER_MAGIC: &[u8; 8] = b"VSHOWN01";
const COMMIT_LOCK_FILE: &str = "commit.lock";

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

struct WorkspaceLockGuard<'a>(&'a File);

impl<'a> WorkspaceLockGuard<'a> {
    fn shared(file: &'a File) -> io::Result<Self> {
        File::lock_shared(file)?;
        Ok(Self(file))
    }

    fn exclusive(file: &'a File) -> io::Result<Self> {
        File::lock(file)?;
        Ok(Self(file))
    }
}

impl Drop for WorkspaceLockGuard<'_> {
    fn drop(&mut self) {
        let _ = File::unlock(self.0);
    }
}

#[derive(Clone, Copy)]
struct OperationWitnesses {
    completed: Option<Witness>,
    source: Option<Witness>,
    parent: Option<Witness>,
}

struct AppliedOperation {
    witness: Witness,
    created_directory: Option<(VPath, Dir)>,
}

/// Hard bounds applied before durable commit work begins.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitConfig {
    /// Maximum journaled filesystem operations in one transaction.
    pub max_operations: usize,
    /// Maximum combined `ReadSet` and `WriteSet` paths.
    pub max_dependencies: usize,
    /// Maximum UTF-8 byte length of one virtual path.
    pub max_path_bytes: usize,
    /// Maximum encoded durable-plan size.
    pub max_plan_bytes: usize,
    /// Maximum journal size accepted during recovery.
    pub max_journal_bytes: usize,
    /// Maximum conflicts returned by one revalidation pass.
    pub max_conflicts: usize,
}

impl Default for CommitConfig {
    fn default() -> Self {
        Self {
            max_operations: 100_000,
            max_dependencies: 250_000,
            max_path_bytes: 16 * 1024,
            max_plan_bytes: 128 * 1024 * 1024,
            max_journal_bytes: 64 * 1024 * 1024,
            max_conflicts: 128,
        }
    }
}

/// Deterministic crash boundary exposed to fault-injection tests.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub enum FaultPoint {
    /// The immutable plan and empty journal are durable.
    PlanSynced,
    /// All replacement content and quarantine directories are durable.
    StageSynced,
    /// Dependency-only revalidation completed successfully.
    Revalidated,
    /// The transaction entered `Committing` durably.
    CommitStatePersisted,
    /// Operation `n`'s intent record is durable.
    IntentSynced(u32),
    /// Operation `n` changed the host but lacks a completion record.
    OperationApplied(u32),
    /// Operation `n` and its ownership witness are durable.
    DoneSynced(u32),
    /// Temporary directory ownership markers were durably removed.
    OwnershipMarkersCleared,
    /// Every final diff path passed content and metadata verification.
    Verified,
    /// The durable final-state marker was synchronized.
    CommitMarkerSynced,
    /// The transaction state became `Committed`.
    CommittedStatePersisted,
}

/// Test seam for simulating process loss at durable boundaries.
pub trait FaultInjector: Send + Sync {
    /// Return `true` to stop at `point` and leave normal recovery artifacts.
    fn should_fail(&self, point: FaultPoint) -> bool;
}

impl<F> FaultInjector for F
where
    F: Fn(FaultPoint) -> bool + Send + Sync,
{
    fn should_fail(&self, point: FaultPoint) -> bool {
        self(point)
    }
}

/// Production fault injector that never interrupts work.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoFaults;

impl FaultInjector for NoFaults {
    fn should_fail(&self, _point: FaultPoint) -> bool {
        false
    }
}

/// One exact dependency mismatch detected before the first host mutation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum RevalidationConflict {
    /// Existence or node metadata changed.
    Metadata {
        /// Conflicting virtual path.
        path: VPath,
        /// State captured by virtual execution.
        expected: Option<NodeState>,
        /// State observed immediately before commit.
        actual: Option<NodeState>,
    },
    /// Exact file or symlink bytes changed.
    Content {
        /// Conflicting virtual path.
        path: VPath,
        /// Content digest captured by virtual execution.
        expected: BlobId,
        /// Current host content digest.
        actual: BlobId,
    },
    /// A direct directory listing changed.
    Directory {
        /// Conflicting directory.
        path: VPath,
        /// Listing digest captured by virtual execution.
        expected: DirectoryDigest,
        /// Current host listing digest.
        actual: DirectoryDigest,
    },
}

/// Compact proof that an exact transaction reached verified durable state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitReceipt {
    /// Committed transaction identity.
    pub transaction: TransactionId,
    /// Number of journaled host operations.
    pub operations: usize,
    /// Number of changed paths verified after apply.
    pub verified_paths: usize,
    /// Whether harmless internal cleanup remains for recovery.
    pub cleanup_pending: bool,
}

/// Aggregate result of scanning durable commit journals.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecoveryReport {
    /// Marker-backed commits whose store state was finalized.
    pub finalized_commits: usize,
    /// Interrupted transactions safely rolled back.
    pub rolled_back: usize,
    /// Internal transaction workspaces removed.
    pub cleaned: usize,
    /// Journals recovered without a matching store record.
    pub orphaned: usize,
    /// Items deliberately left untouched because ownership was ambiguous.
    pub conflicts: Vec<RecoveryConflict>,
}

/// Fail-closed recovery result requiring operator resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryConflict {
    /// Affected transaction.
    pub transaction: TransactionId,
    /// Affected workspace path, when path-specific.
    pub path: Option<VPath>,
    /// Stable conflict explanation.
    pub reason: &'static str,
}

/// Expected and observed state for a failed operation or final-state check.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationFailure {
    /// Path that failed verification.
    pub path: VPath,
    /// State required by the artifact.
    pub expected: Option<NodeState>,
    /// State observed on the host.
    pub actual: Option<NodeState>,
}

/// Trusted-commit, revalidation, or recovery failure.
#[derive(Debug)]
#[non_exhaustive]
pub enum CommitError {
    /// The immutable commit artifact is internally inconsistent.
    Plan(CommitPlanError),
    /// The single-use reservation covers another transaction.
    Binding {
        /// Transaction consumed from the store.
        reserved_transaction: TransactionId,
        /// Transaction derived from the supplied artifact.
        plan_transaction: TransactionId,
    },
    /// Reservation and artifact cover different base snapshots.
    BaseSnapshotBinding,
    /// The combined dependency set exceeds its configured bound.
    DependencyLimit {
        /// Observed dependency count.
        observed: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// An encoded plan or recovery journal exceeds its configured bound.
    PlanSize {
        /// Observed byte length.
        observed: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// A durable workspace already exists and must be recovered first.
    TransactionWorkspaceExists {
        /// Affected transaction.
        transaction: TransactionId,
    },
    /// A caller-supplied blob store overlaps the untrusted workspace namespace.
    UnsafeBlobStore {
        /// Canonical workspace authority root.
        workspace_root: PathBuf,
        /// Canonical immutable blob directory.
        blobs_directory: PathBuf,
    },
    /// Capability-scoped host observation or mutation failed.
    Host(HostError),
    /// Atomic transaction-state persistence failed.
    Store(TransactionStoreError),
    /// Immutable blob loading or verification failed.
    Blob(BlobStoreError),
    /// The protected workspace data capability could not be established.
    DataDirectory(DataDirectoryError),
    /// Durable journal validation failed.
    Journal(JournalError),
    /// Durable plan decoding failed.
    PlanDecode(PlanDecodeError),
    /// Internal capability-directory I/O failed.
    InternalIo {
        /// Stable operation label.
        operation: &'static str,
        /// Underlying host error.
        source: io::Error,
    },
    /// Revalidation detected stale dependencies before mutation.
    Stale {
        /// Bounded set of exact conflicts.
        conflicts: Vec<RevalidationConflict>,
    },
    /// An operation or post-commit final-state check failed.
    Verification(Box<VerificationFailure>),
    /// Test-only simulated crash point fired.
    FaultInjected {
        /// Fired boundary.
        point: FaultPoint,
    },
    /// Mutation may have begun and the durable journal must be recovered.
    RecoveryRequired {
        /// Affected transaction.
        transaction: TransactionId,
        /// Original failure rendered without leaking file content.
        cause: String,
    },
    /// Recovery could not prove ownership and left host data untouched.
    RecoveryConflict(RecoveryConflict),
    /// Store state is incompatible with the durable recovery artifact.
    InvalidRecoveryState {
        /// Affected transaction.
        transaction: TransactionId,
        /// Unexpected persisted state.
        state: TransactionState,
    },
}

impl fmt::Display for CommitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(source) => fmt::Display::fmt(source, formatter),
            Self::Binding {
                reserved_transaction,
                plan_transaction,
            } => write!(
                formatter,
                "commit reservation {reserved_transaction} does not bind plan {plan_transaction}"
            ),
            Self::BaseSnapshotBinding => {
                formatter.write_str("commit reservation and plan bind different base snapshots")
            }
            Self::DependencyLimit { observed, maximum } => write!(
                formatter,
                "commit has {observed} dependencies; maximum is {maximum}"
            ),
            Self::PlanSize { observed, maximum } => {
                write!(
                    formatter,
                    "commit plan is {observed} bytes; maximum is {maximum}"
                )
            }
            Self::TransactionWorkspaceExists { transaction } => write!(
                formatter,
                "transaction workspace already exists for {transaction}; recovery is required"
            ),
            Self::UnsafeBlobStore {
                workspace_root,
                blobs_directory,
            } => write!(
                formatter,
                "blob store {} must be disjoint from workspace {}",
                blobs_directory.display(),
                workspace_root.display()
            ),
            Self::Host(source) => fmt::Display::fmt(source, formatter),
            Self::Store(source) => fmt::Display::fmt(source, formatter),
            Self::Blob(source) => fmt::Display::fmt(source, formatter),
            Self::DataDirectory(source) => fmt::Display::fmt(source, formatter),
            Self::Journal(source) => fmt::Display::fmt(source, formatter),
            Self::PlanDecode(source) => fmt::Display::fmt(source, formatter),
            Self::InternalIo { operation, source } => write!(formatter, "{operation}: {source}"),
            Self::Stale { conflicts } => write!(
                formatter,
                "commit dependencies are stale ({} conflict(s))",
                conflicts.len()
            ),
            Self::Verification(failure) => {
                write!(
                    formatter,
                    "post-commit verification failed at {}",
                    failure.path
                )
            }
            Self::FaultInjected { point } => write!(formatter, "injected fault at {point:?}"),
            Self::RecoveryRequired { transaction, cause } => {
                write!(
                    formatter,
                    "transaction {transaction} requires recovery: {cause}"
                )
            }
            Self::RecoveryConflict(conflict) => write!(
                formatter,
                "transaction {} recovery conflict: {}",
                conflict.transaction, conflict.reason
            ),
            Self::InvalidRecoveryState { transaction, state } => write!(
                formatter,
                "transaction {transaction} has invalid recovery state {state:?}"
            ),
        }
    }
}

impl Error for CommitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Plan(source) => Some(source),
            Self::Host(source) => Some(source),
            Self::Store(source) => Some(source),
            Self::Blob(source) => Some(source),
            Self::DataDirectory(source) => Some(source),
            Self::Journal(source) => Some(source),
            Self::PlanDecode(source) => Some(source),
            Self::InternalIo { source, .. } => Some(source),
            Self::Binding { .. }
            | Self::BaseSnapshotBinding
            | Self::DependencyLimit { .. }
            | Self::PlanSize { .. }
            | Self::TransactionWorkspaceExists { .. }
            | Self::UnsafeBlobStore { .. }
            | Self::Stale { .. }
            | Self::Verification(_)
            | Self::FaultInjected { .. }
            | Self::RecoveryRequired { .. }
            | Self::RecoveryConflict(_)
            | Self::InvalidRecoveryState { .. } => None,
        }
    }
}

impl From<CommitPlanError> for CommitError {
    fn from(source: CommitPlanError) -> Self {
        Self::Plan(source)
    }
}

impl From<HostError> for CommitError {
    fn from(source: HostError) -> Self {
        Self::Host(source)
    }
}

impl From<TransactionStoreError> for CommitError {
    fn from(source: TransactionStoreError) -> Self {
        Self::Store(source)
    }
}

impl From<BlobStoreError> for CommitError {
    fn from(source: BlobStoreError) -> Self {
        Self::Blob(source)
    }
}

impl From<DataDirectoryError> for CommitError {
    fn from(source: DataDirectoryError) -> Self {
        Self::DataDirectory(source)
    }
}

impl From<JournalError> for CommitError {
    fn from(source: JournalError) -> Self {
        Self::Journal(source)
    }
}

impl From<PlanDecodeError> for CommitError {
    fn from(source: PlanDecodeError) -> Self {
        Self::PlanDecode(source)
    }
}

/// Capability-rooted workspace snapshot, revalidation, commit, and recovery engine.
pub struct Committer {
    workspace_root: Arc<PathBuf>,
    root: Arc<Dir>,
    root_file_id: PlatformFileId,
    runtime: Arc<Dir>,
    runtime_file_id: PlatformFileId,
    transactions: Arc<Dir>,
    coordination: Arc<File>,
    blobs: BlobStore,
    config: CommitConfig,
}

impl Committer {
    /// Open one ambient workspace boundary and create its protected runtime directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the root cannot be opened or a reserved internal path is not
    /// a real directory.
    pub fn open(
        workspace_root: impl AsRef<Path>,
        blobs: BlobStore,
        config: CommitConfig,
    ) -> Result<Self, CommitError> {
        let workspace_root = fs::canonicalize(workspace_root.as_ref()).map_err(|source| {
            CommitError::InternalIo {
                operation: "canonicalize workspace capability",
                source,
            }
        })?;
        if paths_overlap(&workspace_root, blobs.blobs_dir()) {
            return Err(CommitError::UnsafeBlobStore {
                workspace_root,
                blobs_directory: blobs.blobs_dir().to_path_buf(),
            });
        }
        let (root, root_file_id, runtime, runtime_file_id, transactions, coordination) =
            Self::open_workspace_components(&workspace_root)?;
        let committer = Self {
            workspace_root: Arc::new(workspace_root),
            root: Arc::new(root),
            root_file_id,
            runtime: Arc::new(runtime),
            runtime_file_id,
            transactions: Arc::new(transactions),
            coordination: Arc::new(coordination),
            blobs,
            config,
        };
        committer.validate_runtime_directory()?;
        Ok(committer)
    }

    /// Open one workspace and derive its committer and durable data store from the
    /// same pinned `.vsh-runtime` directory capability.
    ///
    /// # Errors
    ///
    /// Returns an error if any protected directory, coordination file, or blob store
    /// cannot be created and verified without following a workspace symlink.
    pub fn open_with_workspace_data(
        workspace_root: impl AsRef<Path>,
        config: CommitConfig,
    ) -> Result<(Self, DataDirectory), CommitError> {
        let workspace_root = fs::canonicalize(workspace_root.as_ref()).map_err(|source| {
            CommitError::InternalIo {
                operation: "canonicalize workspace capability",
                source,
            }
        })?;
        let (root, root_file_id, runtime, runtime_file_id, transactions, coordination) =
            Self::open_workspace_components(&workspace_root)?;
        let data_directory = DataDirectory::open_runtime_data(&runtime, &workspace_root)?;
        let blobs = BlobStore::open_in(&data_directory)?;
        sync_dir(&runtime).map_err(|source| CommitError::InternalIo {
            operation: "sync VSH runtime data directory",
            source,
        })?;
        sync_dir(&root).map_err(|source| CommitError::InternalIo {
            operation: "sync workspace data parent",
            source,
        })?;
        let committer = Self {
            workspace_root: Arc::new(workspace_root),
            root: Arc::new(root),
            root_file_id,
            runtime: Arc::new(runtime),
            runtime_file_id,
            transactions: Arc::new(transactions),
            coordination: Arc::new(coordination),
            blobs,
            config,
        };
        committer.validate_runtime_directory()?;
        Ok((committer, data_directory))
    }

    /// Return a cheap handle to the immutable artifact store owned by this committer.
    #[must_use]
    pub fn artifact_store(&self) -> BlobStore {
        self.blobs.clone()
    }

    fn open_workspace_components(
        workspace_root: &Path,
    ) -> Result<(Dir, PlatformFileId, Dir, PlatformFileId, Dir, File), CommitError> {
        let root =
            Dir::open_ambient_dir(workspace_root, ambient_authority()).map_err(|source| {
                CommitError::InternalIo {
                    operation: "open workspace capability",
                    source,
                }
            })?;
        let root_stamp = stamp_dir(&root, &VPath::root())?;
        let runtime =
            open_or_create_real_dir(&root, crate::host::RUNTIME_DIRECTORY).map_err(|source| {
                CommitError::InternalIo {
                    operation: "open VSH runtime directory",
                    source,
                }
            })?;
        let runtime_path = VPath::parse(crate::host::RUNTIME_DIRECTORY)
            .expect("built-in runtime directory is a valid VPath");
        let opened = stamp_dir(&runtime, &runtime_path)?;
        let named = stamp_at(&root, &runtime_path)?.ok_or_else(|| {
            HostError::io(
                "verify VSH runtime directory",
                &runtime_path,
                io::Error::new(io::ErrorKind::NotFound, "runtime directory disappeared"),
            )
        })?;
        if opened.kind != NodeKind::Directory
            || named.kind != NodeKind::Directory
            || opened.file_id != named.file_id
        {
            return Err(HostError::Unstable {
                path: runtime_path,
                before: Box::new(named),
                after: Box::new(opened),
            }
            .into());
        }
        let transactions = open_or_create_real_dir(&runtime, crate::host::TRANSACTIONS_DIRECTORY)
            .map_err(|source| CommitError::InternalIo {
            operation: "open VSH transaction directory",
            source,
        })?;
        let coordination = open_coordination_file(&runtime, COMMIT_LOCK_FILE)?;
        sync_dir(&runtime).map_err(|source| CommitError::InternalIo {
            operation: "sync VSH runtime directory",
            source,
        })?;
        sync_dir(&root).map_err(|source| CommitError::InternalIo {
            operation: "sync workspace root",
            source,
        })?;
        Ok((
            root,
            root_stamp.file_id,
            runtime,
            opened.file_id,
            transactions,
            coordination,
        ))
    }

    fn validate_workspace_directory(&self) -> Result<(), CommitError> {
        let metadata = fs::symlink_metadata(self.workspace_root.as_path()).map_err(|source| {
            CommitError::InternalIo {
                operation: "inspect named workspace directory",
                source,
            }
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(CommitError::InternalIo {
                operation: "verify pinned workspace directory",
                source: io::Error::new(
                    io::ErrorKind::InvalidData,
                    "workspace path is no longer a real directory",
                ),
            });
        }
        let named = Dir::open_ambient_dir(self.workspace_root.as_path(), ambient_authority())
            .map_err(|source| CommitError::InternalIo {
                operation: "reopen named workspace directory",
                source,
            })?;
        let stamp = stamp_dir(&named, &VPath::root())?;
        let final_metadata =
            fs::symlink_metadata(self.workspace_root.as_path()).map_err(|source| {
                CommitError::InternalIo {
                    operation: "reinspect named workspace directory",
                    source,
                }
            })?;
        if stamp.kind == NodeKind::Directory
            && stamp.file_id == self.root_file_id
            && final_metadata.is_dir()
            && !final_metadata.file_type().is_symlink()
        {
            Ok(())
        } else {
            Err(CommitError::InternalIo {
                operation: "verify pinned workspace directory",
                source: io::Error::new(
                    io::ErrorKind::InvalidData,
                    "workspace directory identity changed",
                ),
            })
        }
    }

    fn validate_runtime_directory(&self) -> Result<(), CommitError> {
        self.validate_workspace_directory()?;
        let runtime_path = VPath::parse(crate::host::RUNTIME_DIRECTORY)
            .expect("built-in runtime directory is a valid VPath");
        let opened = stamp_dir(&self.runtime, &runtime_path)?;
        let named = stamp_at(&self.root, &runtime_path)?;
        if opened.kind == NodeKind::Directory
            && opened.file_id == self.runtime_file_id
            && named.is_some_and(|named| {
                named.kind == NodeKind::Directory && named.file_id == self.runtime_file_id
            })
        {
            Ok(())
        } else {
            Err(CommitError::InternalIo {
                operation: "verify pinned VSH runtime directory",
                source: io::Error::new(
                    io::ErrorKind::InvalidData,
                    "protected runtime directory identity changed",
                ),
            })
        }
    }

    #[must_use]
    /// Return immutable commit bounds.
    pub const fn config(&self) -> CommitConfig {
        self.config
    }

    /// Capture an eager-metadata, lazy-content snapshot below the workspace capability.
    ///
    /// # Errors
    ///
    /// Returns an error for unsupported nodes, unstable enumeration, or a size bound.
    pub fn snapshot(&self, limits: SnapshotLimits) -> Result<BaseSnapshot, CommitError> {
        let _guard = WorkspaceLockGuard::shared(&self.coordination).map_err(|source| {
            CommitError::InternalIo {
                operation: "acquire shared workspace lock",
                source,
            }
        })?;
        self.validate_runtime_directory()?;
        let snapshot = capture_snapshot(&self.root, self.blobs.clone(), limits).map_err(Into::into);
        self.validate_runtime_directory()?;
        snapshot
    }

    /// Revalidate exactly the artifact's `ReadSet` and `WriteSet` without mutating the host.
    ///
    /// # Errors
    ///
    /// Returns an error when safe host observation fails or bounds are exceeded.
    pub fn revalidate(
        &self,
        plan: &CommitPlan<'_>,
    ) -> Result<Vec<RevalidationConflict>, CommitError> {
        self.validate_runtime_directory()?;
        let dependency_count = plan.read_set().len().saturating_add(plan.write_set().len());
        if dependency_count > self.config.max_dependencies {
            return Err(CommitError::DependencyLimit {
                observed: dependency_count,
                maximum: self.config.max_dependencies,
            });
        }
        let mut conflicts = Vec::new();
        for (path, observation) in plan.read_set() {
            self.revalidate_read(path, observation, &mut conflicts)?;
            if conflicts.len() >= self.config.max_conflicts {
                self.validate_runtime_directory()?;
                return Ok(conflicts);
            }
        }
        for (path, precondition) in plan.write_set() {
            let (matches, actual) = state_matches(&self.root, path, precondition.expected)?;
            if !matches {
                conflicts.push(RevalidationConflict::Metadata {
                    path: path.clone(),
                    expected: precondition.expected,
                    actual,
                });
                if conflicts.len() >= self.config.max_conflicts {
                    break;
                }
            }
        }
        self.validate_runtime_directory()?;
        Ok(conflicts)
    }

    /// Consume a reservation and commit one exact artifact with production fault policy.
    ///
    /// # Errors
    ///
    /// Returns a stale, binding, I/O, or recovery-required error. Once mutation begins,
    /// failures preserve durable recovery obligations.
    pub fn commit<S: TransactionStore + ?Sized>(
        &self,
        store: &S,
        reservation: CommitReservation,
        plan: &CommitPlan<'_>,
    ) -> Result<CommitReceipt, CommitError> {
        self.commit_with_faults(store, reservation, plan, &NoFaults)
    }

    /// Commit with an explicit durable-boundary fault injector.
    ///
    /// This is public so downstream crash harnesses can validate filesystem/platform
    /// behavior without private hooks.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::commit`], plus injected failures.
    #[allow(clippy::needless_pass_by_value)]
    pub fn commit_with_faults<S, F>(
        &self,
        store: &S,
        reservation: CommitReservation,
        plan: &CommitPlan<'_>,
        faults: &F,
    ) -> Result<CommitReceipt, CommitError>
    where
        S: TransactionStore + ?Sized,
        F: FaultInjector + ?Sized,
    {
        let (transaction, prepared, encoded) =
            self.prepare_reserved_plan(store, &reservation, plan)?;
        let _guard = WorkspaceLockGuard::exclusive(&self.coordination).map_err(|source| {
            Self::fail_reserved(
                store,
                transaction,
                CommitError::InternalIo {
                    operation: "acquire exclusive workspace lock",
                    source,
                },
            )
        })?;
        self.validate_runtime_directory()
            .map_err(|error| Self::fail_reserved(store, transaction, error))?;
        store.compare_and_transition(
            transaction,
            TransactionState::Reserved,
            TransactionState::Revalidating,
        )?;

        let transaction_name = transaction.to_string();
        let transaction_dir = match self.create_transaction_workspace(&transaction_name) {
            Ok(directory) => directory,
            Err(error) => {
                let _ = store.compare_and_transition(
                    transaction,
                    TransactionState::Revalidating,
                    TransactionState::Failed,
                );
                return Err(error);
            }
        };
        let result = self.prepare_and_commit(
            store,
            transaction,
            &transaction_dir,
            plan,
            &prepared,
            &encoded,
            faults,
        );
        // Windows capability directories intentionally deny rename/delete while
        // open, so the transaction root must close before cleanup is attempted.
        drop(transaction_dir);
        self.resolve_commit_result(store, transaction, &transaction_name, &prepared, result)
    }

    fn prepare_reserved_plan<S: TransactionStore + ?Sized>(
        &self,
        store: &S,
        reservation: &CommitReservation,
        plan: &CommitPlan<'_>,
    ) -> Result<(TransactionId, PreparedPlan, Vec<u8>), CommitError> {
        let transaction = plan.transaction();
        let reserved_transaction = reservation.transaction();
        if reserved_transaction != transaction {
            return Err(Self::fail_reserved(
                store,
                reserved_transaction,
                CommitError::Binding {
                    reserved_transaction,
                    plan_transaction: transaction,
                },
            ));
        }
        if reservation.base_snapshot() != plan.base_snapshot() {
            return Err(Self::fail_reserved(
                store,
                reserved_transaction,
                CommitError::BaseSnapshotBinding,
            ));
        }
        let prepared =
            PreparedPlan::prepare(plan, self.config.max_operations, self.config.max_path_bytes)
                .map_err(|source| {
                    Self::fail_reserved(store, reserved_transaction, CommitError::from(source))
                })?;
        let encoded = prepared.encode().map_err(|source| {
            Self::fail_reserved(store, reserved_transaction, CommitError::from(source))
        })?;
        if encoded.len() > self.config.max_plan_bytes {
            return Err(Self::fail_reserved(
                store,
                reserved_transaction,
                CommitError::PlanSize {
                    observed: encoded.len(),
                    maximum: self.config.max_plan_bytes,
                },
            ));
        }
        Ok((transaction, prepared, encoded))
    }

    fn resolve_commit_result<S: TransactionStore + ?Sized>(
        &self,
        store: &S,
        transaction: TransactionId,
        transaction_name: &str,
        prepared: &PreparedPlan,
        result: Result<CommitReceipt, CommitError>,
    ) -> Result<CommitReceipt, CommitError> {
        match result {
            Ok(mut receipt) => {
                receipt.cleanup_pending = self.cleanup_transaction(transaction_name).is_err();
                Ok(receipt)
            }
            Err(error) => {
                let current = store.get(transaction).ok().map(|record| record.state());
                match current {
                    Some(TransactionState::Revalidating) => {
                        let _ = store.compare_and_transition(
                            transaction,
                            TransactionState::Revalidating,
                            TransactionState::Failed,
                        );
                        let _ = self.cleanup_transaction(transaction_name);
                        Err(error)
                    }
                    Some(TransactionState::Committing) => {
                        let _ = store.compare_and_transition(
                            transaction,
                            TransactionState::Committing,
                            TransactionState::RecoveryRequired,
                        );
                        Err(CommitError::RecoveryRequired {
                            transaction,
                            cause: error.to_string(),
                        })
                    }
                    Some(TransactionState::RecoveryRequired) => {
                        Err(CommitError::RecoveryRequired {
                            transaction,
                            cause: error.to_string(),
                        })
                    }
                    Some(TransactionState::Committed) => {
                        let cleanup_pending = self.cleanup_transaction(transaction_name).is_err();
                        Ok(CommitReceipt {
                            transaction,
                            operations: prepared.operations.len(),
                            verified_paths: prepared.final_states.len(),
                            cleanup_pending,
                        })
                    }
                    Some(TransactionState::Stale) => {
                        let _ = self.cleanup_transaction(transaction_name);
                        Err(error)
                    }
                    _ => Err(error),
                }
            }
        }
    }

    fn fail_reserved<S: TransactionStore + ?Sized>(
        store: &S,
        transaction: TransactionId,
        error: CommitError,
    ) -> CommitError {
        match store.compare_and_transition(
            transaction,
            TransactionState::Reserved,
            TransactionState::Failed,
        ) {
            Ok(_) => error,
            Err(source) => CommitError::Store(source),
        }
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn prepare_and_commit<S, F>(
        &self,
        store: &S,
        transaction: TransactionId,
        transaction_dir: &Dir,
        plan: &CommitPlan<'_>,
        prepared: &PreparedPlan,
        encoded: &[u8],
        faults: &F,
    ) -> Result<CommitReceipt, CommitError>
    where
        S: TransactionStore + ?Sized,
        F: FaultInjector + ?Sized,
    {
        Self::write_plan(transaction_dir, encoded)?;
        let mut journal =
            Journal::create(transaction_dir).map_err(|source| CommitError::InternalIo {
                operation: "create commit journal",
                source,
            })?;
        Self::check_fault(faults, FaultPoint::PlanSynced)?;

        let stage =
            open_or_create_real_dir(transaction_dir, STAGE_DIRECTORY).map_err(|source| {
                CommitError::InternalIo {
                    operation: "create commit staging directory",
                    source,
                }
            })?;
        let quarantine =
            open_or_create_real_dir(transaction_dir, QUARANTINE_DIRECTORY).map_err(|source| {
                CommitError::InternalIo {
                    operation: "create commit quarantine directory",
                    source,
                }
            })?;
        self.stage_content(prepared, &stage)?;
        sync_dir(&stage).map_err(|source| CommitError::InternalIo {
            operation: "sync commit staging directory",
            source,
        })?;
        sync_dir(&quarantine).map_err(|source| CommitError::InternalIo {
            operation: "sync commit quarantine directory",
            source,
        })?;
        sync_dir(transaction_dir).map_err(|source| CommitError::InternalIo {
            operation: "sync transaction directory",
            source,
        })?;
        Self::check_fault(faults, FaultPoint::StageSynced)?;

        let conflicts = self.revalidate(plan)?;
        if !conflicts.is_empty() {
            store.compare_and_transition(
                transaction,
                TransactionState::Revalidating,
                TransactionState::Stale,
            )?;
            return Err(CommitError::Stale { conflicts });
        }
        let mut pinned_parents = match self.pin_parent_directories(plan, prepared) {
            Ok(parents) => parents,
            Err(CommitError::Stale { conflicts }) => {
                store.compare_and_transition(
                    transaction,
                    TransactionState::Revalidating,
                    TransactionState::Stale,
                )?;
                return Err(CommitError::Stale { conflicts });
            }
            Err(error) => return Err(error),
        };
        Self::check_fault(faults, FaultPoint::Revalidated)?;
        store.compare_and_transition(
            transaction,
            TransactionState::Revalidating,
            TransactionState::Committing,
        )?;
        Self::check_fault(faults, FaultPoint::CommitStatePersisted)?;

        let mut completed_witnesses = Vec::with_capacity(prepared.operations.len());
        for (index, operation) in prepared.operations.iter().enumerate() {
            let index =
                u32::try_from(index).map_err(|_| CommitPlanError::OperationCountOverflow)?;
            let parent_path = operation
                .path()
                .parent()
                .expect("commit operations cannot target the workspace root");
            let parent =
                pinned_parents
                    .get(&parent_path)
                    .ok_or_else(|| CommitError::InternalIo {
                        operation: "locate pinned commit parent",
                        source: io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("commit parent {parent_path} was not pinned"),
                        ),
                    })?;
            let source_witness = Self::operation_source_witness(operation, &stage)?;
            let parent_witness = Witness::from(stamp_dir(parent, &parent_path)?);
            journal
                .intent(index, source_witness, parent_witness)
                .map_err(|source| CommitError::InternalIo {
                    operation: "sync commit intent",
                    source,
                })?;
            Self::check_fault(faults, FaultPoint::IntentSynced(index))?;
            let applied =
                Self::apply_operation(transaction, index, operation, parent, &stage, &quarantine)?;
            if let Some((path, directory)) = applied.created_directory {
                pinned_parents.insert(path, directory);
            }
            Self::check_fault(faults, FaultPoint::OperationApplied(index))?;
            journal
                .done(index, applied.witness)
                .map_err(|source| CommitError::InternalIo {
                    operation: "sync commit completion",
                    source,
                })?;
            completed_witnesses.push(applied.witness);
            Self::check_fault(faults, FaultPoint::DoneSynced(index))?;
        }
        for (index, operation) in prepared.operations.iter().enumerate().rev() {
            let index =
                u32::try_from(index).map_err(|_| CommitPlanError::OperationCountOverflow)?;
            Self::clear_operation_marker(
                transaction,
                index,
                operation,
                &pinned_parents,
                completed_witnesses[usize::try_from(index).expect("u32 fits usize")],
            )?;
        }
        Self::check_fault(faults, FaultPoint::OwnershipMarkersCleared)?;
        self.verify_final(prepared)?;
        self.validate_runtime_directory()?;
        Self::check_fault(faults, FaultPoint::Verified)?;
        write_commit_marker(transaction_dir, transaction).map_err(|source| {
            CommitError::InternalIo {
                operation: "sync commit-complete marker",
                source,
            }
        })?;
        Self::check_fault(faults, FaultPoint::CommitMarkerSynced)?;
        self.validate_runtime_directory()?;
        store.compare_and_transition(
            transaction,
            TransactionState::Committing,
            TransactionState::Committed,
        )?;
        Self::check_fault(faults, FaultPoint::CommittedStatePersisted)?;
        Ok(CommitReceipt {
            transaction,
            operations: prepared.operations.len(),
            verified_paths: prepared.final_states.len(),
            // The outer commit frame retries after all transaction handles close.
            cleanup_pending: true,
        })
    }

    fn pin_parent_directories(
        &self,
        plan: &CommitPlan<'_>,
        prepared: &PreparedPlan,
    ) -> Result<BTreeMap<VPath, Dir>, CommitError> {
        let parents = prepared
            .operations
            .iter()
            .map(|operation| {
                operation
                    .path()
                    .parent()
                    .expect("commit operations cannot target the workspace root")
            })
            .collect::<BTreeSet<_>>();
        let mut pinned = BTreeMap::new();
        let mut conflicts = Vec::new();
        for parent in parents {
            let expected = plan
                .read_set()
                .get(&parent)
                .and_then(|observation| observation.metadata)
                .expect("validated commit plans have parent metadata dependencies");
            let Some(expected) = expected else {
                continue;
            };
            let directory = if parent.is_root() {
                self.root.try_clone().map_err(|source| {
                    HostError::io("pin workspace root for commit", &parent, source)
                })?
            } else if let Ok(directory) = self.root.open_dir(relative_path(&parent)) {
                directory
            } else {
                conflicts.push(RevalidationConflict::Metadata {
                    path: parent.clone(),
                    expected: Some(expected),
                    actual: stamp_at(&self.root, &parent)?.map(NodeState::from_stamp),
                });
                continue;
            };
            let actual = NodeState::from_stamp(stamp_dir(&directory, &parent)?);
            if actual != expected {
                conflicts.push(RevalidationConflict::Metadata {
                    path: parent,
                    expected: Some(expected),
                    actual: Some(actual),
                });
                continue;
            }
            pinned.insert(parent, directory);
        }
        if conflicts.is_empty() {
            Ok(pinned)
        } else {
            Err(CommitError::Stale { conflicts })
        }
    }

    fn create_transaction_workspace(&self, name: &str) -> Result<Dir, CommitError> {
        match self.transactions.create_dir(name) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                let transaction = parse_transaction_name(name)
                    .unwrap_or_else(|| TransactionId::from_bytes([0; 32]));
                return Err(CommitError::TransactionWorkspaceExists { transaction });
            }
            Err(source) => {
                return Err(CommitError::InternalIo {
                    operation: "create transaction workspace",
                    source,
                });
            }
        }
        sync_dir(&self.transactions).map_err(|source| CommitError::InternalIo {
            operation: "sync transaction workspace parent",
            source,
        })?;
        open_real_dir(&self.transactions, name).map_err(|source| CommitError::InternalIo {
            operation: "open transaction workspace",
            source,
        })
    }

    fn write_plan(transaction_dir: &Dir, bytes: &[u8]) -> Result<(), CommitError> {
        let mut file = create_new_file(transaction_dir, PLAN_FILE).map_err(|source| {
            CommitError::InternalIo {
                operation: "create durable commit plan",
                source,
            }
        })?;
        file.write_all(bytes)
            .map_err(|source| CommitError::InternalIo {
                operation: "write durable commit plan",
                source,
            })?;
        file.sync_all().map_err(|source| CommitError::InternalIo {
            operation: "sync durable commit plan",
            source,
        })?;
        sync_dir(transaction_dir).map_err(|source| CommitError::InternalIo {
            operation: "sync durable commit-plan directory",
            source,
        })
    }

    fn stage_content(&self, plan: &PreparedPlan, stage: &Dir) -> Result<(), CommitError> {
        for operation in &plan.operations {
            let (after, slot) = match operation {
                Operation::InstallFile { after, slot, .. }
                | Operation::InstallSymlink { after, slot, .. } => (*after, *slot),
                Operation::Quarantine { .. }
                | Operation::CreateDirectory { .. }
                | Operation::SetDirectoryMode { .. } => continue,
            };
            let Some(ContentVersion::Blob(blob)) = after.content() else {
                return Err(CommitError::Plan(
                    CommitPlanError::UnmaterializedAfterState {
                        path: operation.path().clone(),
                    },
                ));
            };
            let bytes = self.blobs.get(blob)?;
            if bytes.len() as u64 != after.size() {
                return Err(CommitError::Blob(BlobStoreError::Corrupt {
                    path: self.blobs.blobs_dir().to_owned(),
                    expected: blob,
                    actual: BlobId::digest(&bytes),
                }));
            }
            let name = stage_name(slot);
            let mut file =
                create_new_file(stage, &name).map_err(|source| CommitError::InternalIo {
                    operation: "create staged content",
                    source,
                })?;
            file.write_all(&bytes)
                .map_err(|source| CommitError::InternalIo {
                    operation: "write staged content",
                    source,
                })?;
            if after.kind() == NodeKind::File {
                set_file_mode(&file, after.mode()).map_err(|source| CommitError::InternalIo {
                    operation: "set staged file mode",
                    source,
                })?;
            }
            file.sync_all().map_err(|source| CommitError::InternalIo {
                operation: "sync staged content",
                source,
            })?;
            if after.kind() == NodeKind::Symlink {
                let target = validate_symlink_target(operation.path(), &bytes)?;
                create_staged_symlink(
                    stage,
                    &stage_link_name(slot),
                    &self.root,
                    operation.path(),
                    &target,
                )?;
            }
        }
        Ok(())
    }

    fn operation_source_witness(
        operation: &Operation,
        stage: &Dir,
    ) -> Result<Option<Witness>, CommitError> {
        let Operation::InstallSymlink { path, slot, .. } = operation else {
            return Ok(None);
        };
        let staged_path = VPath::parse(&stage_link_name(*slot))
            .expect("staged symbolic-link name is a valid VPath");
        let stamp = stamp_at(stage, &staged_path)?.ok_or_else(|| {
            CommitError::Host(HostError::io(
                "inspect staged symbolic link",
                path,
                io::Error::new(io::ErrorKind::NotFound, "staged symbolic link is missing"),
            ))
        })?;
        Ok(Some(stamp.into()))
    }

    fn revalidate_read(
        &self,
        path: &VPath,
        observation: &ReadObservation,
        conflicts: &mut Vec<RevalidationConflict>,
    ) -> Result<(), CommitError> {
        if let Some(expected) = observation.metadata {
            let (matches, actual) = state_matches(&self.root, path, expected)?;
            if !matches {
                conflicts.push(RevalidationConflict::Metadata {
                    path: path.clone(),
                    expected,
                    actual,
                });
                return Ok(());
            }
        }
        if let Some(expected) = observation.content {
            let actual = content_digest(&self.root, path)?;
            if actual != expected {
                conflicts.push(RevalidationConflict::Content {
                    path: path.clone(),
                    expected,
                    actual,
                });
                return Ok(());
            }
        }
        if let Some(expected) = observation.directory {
            let actual = directory_digest(&self.root, path, self.config.max_dependencies)?;
            if actual != expected {
                conflicts.push(RevalidationConflict::Directory {
                    path: path.clone(),
                    expected,
                    actual,
                });
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn apply_operation(
        transaction: TransactionId,
        index: u32,
        operation: &Operation,
        parent: &Dir,
        stage: &Dir,
        quarantine: &Dir,
    ) -> Result<AppliedOperation, CommitError> {
        let path = operation.path();
        let leaf = path
            .file_name()
            .expect("commit operations cannot target the workspace root");
        let leaf_path = VPath::parse(leaf).expect("a VPath leaf is a valid VPath");
        match operation {
            Operation::Quarantine {
                path,
                expected,
                slot,
            } => {
                let (matches, _) = state_matches(parent, &leaf_path, Some(*expected))?;
                if !matches {
                    return Err(CommitError::Verification(Box::new(VerificationFailure {
                        path: path.clone(),
                        expected: Some(*expected),
                        actual: stamp_at(parent, &leaf_path)?.map(NodeState::from_stamp),
                    })));
                }
                let name = quarantine_name(*slot);
                parent
                    .rename(leaf, quarantine, &name)
                    .map_err(|source| HostError::io("move old node to quarantine", path, source))?;
                let quarantine_path =
                    VPath::parse(&name).expect("quarantine slot is a valid VPath");
                if !relocated_state_matches(quarantine, &quarantine_path, *expected)? {
                    return Err(CommitError::Verification(Box::new(VerificationFailure {
                        path: path.clone(),
                        expected: Some(*expected),
                        actual: stamp_at(quarantine, &quarantine_path)?.map(NodeState::from_stamp),
                    })));
                }
                sync_dir(parent).map_err(|source| {
                    HostError::io("sync committed parent directory", path, source)
                })?;
                sync_dir(quarantine).map_err(|source| CommitError::InternalIo {
                    operation: "sync quarantine directory",
                    source,
                })?;
                let stamp = stamp_at(quarantine, &quarantine_path)?
                    .expect("verified quarantined node remains present");
                Ok(AppliedOperation {
                    witness: stamp.into(),
                    created_directory: None,
                })
            }
            Operation::CreateDirectory { path, after } => {
                parent
                    .create_dir(leaf)
                    .map_err(|source| HostError::io("create committed directory", path, source))?;
                let directory = parent
                    .open_dir(leaf)
                    .map_err(|source| HostError::io("open committed directory", path, source))?;
                set_dir_mode(&directory, after.mode()).map_err(|source| {
                    HostError::io("set committed directory mode", path, source)
                })?;
                sync_dir(&directory)
                    .map_err(|source| HostError::io("sync committed directory", path, source))?;
                Self::write_directory_owner(&directory, transaction, index).map_err(|source| {
                    HostError::io("write directory ownership marker", path, source)
                })?;
                sync_dir(parent).map_err(|source| {
                    HostError::io("sync committed parent directory", path, source)
                })?;
                Ok(AppliedOperation {
                    witness: stamp_dir(&directory, path)?.into(),
                    created_directory: Some((path.clone(), directory)),
                })
            }
            Operation::InstallFile { path, after, slot } => {
                let name = stage_name(*slot);
                stage
                    .hard_link(&name, parent, leaf)
                    .map_err(|source| HostError::io("install committed file", path, source))?;
                let (matches, actual) = state_matches(parent, &leaf_path, Some(*after))?;
                if !matches {
                    return Err(CommitError::Verification(Box::new(VerificationFailure {
                        path: path.clone(),
                        expected: Some(*after),
                        actual,
                    })));
                }
                let file = parent
                    .open(leaf)
                    .map_err(|source| HostError::io("open committed file", path, source))?;
                sync_installed_file(&file)
                    .map_err(|source| HostError::io("sync committed file", path, source))?;
                sync_dir(parent).map_err(|source| {
                    HostError::io("sync committed parent directory", path, source)
                })?;
                Ok(AppliedOperation {
                    witness: stamp_file(&file, path)?.into(),
                    created_directory: None,
                })
            }
            Operation::InstallSymlink { path, after, slot } => {
                stage
                    .rename(stage_link_name(*slot), parent, leaf)
                    .map_err(|source| {
                        HostError::io("install committed symbolic link", path, source)
                    })?;
                let (matches, actual) = state_matches(parent, &leaf_path, Some(*after))?;
                if !matches {
                    return Err(CommitError::Verification(Box::new(VerificationFailure {
                        path: path.clone(),
                        expected: Some(*after),
                        actual,
                    })));
                }
                sync_dir(parent).map_err(|source| {
                    HostError::io("sync committed parent directory", path, source)
                })?;
                Ok(AppliedOperation {
                    witness: stamp_at(parent, &leaf_path)?
                        .expect("verified committed symlink remains present")
                        .into(),
                    created_directory: None,
                })
            }
            Operation::SetDirectoryMode {
                path,
                expected,
                after_mode,
            } => {
                let (matches, actual) = state_matches(parent, &leaf_path, Some(*expected))?;
                if !matches {
                    return Err(CommitError::Verification(Box::new(VerificationFailure {
                        path: path.clone(),
                        expected: Some(*expected),
                        actual,
                    })));
                }
                let directory = parent.open_dir(leaf).map_err(|source| {
                    HostError::io("open directory for metadata commit", path, source)
                })?;
                set_dir_mode(&directory, *after_mode).map_err(|source| {
                    HostError::io("set committed directory mode", path, source)
                })?;
                sync_dir(&directory).map_err(|source| {
                    HostError::io("sync committed directory mode", path, source)
                })?;
                sync_dir(parent).map_err(|source| {
                    HostError::io("sync committed parent directory", path, source)
                })?;
                Ok(AppliedOperation {
                    witness: stamp_dir(&directory, path)?.into(),
                    created_directory: None,
                })
            }
        }
    }

    fn write_directory_owner(
        directory: &Dir,
        transaction: TransactionId,
        index: u32,
    ) -> Result<(), io::Error> {
        let mut file = create_new_file(directory, DIRECTORY_OWNER_MARKER)?;
        file.write_all(&directory_owner_payload(transaction, index))?;
        file.sync_all()?;
        sync_dir(directory)
    }

    fn clear_operation_marker(
        transaction: TransactionId,
        index: u32,
        operation: &Operation,
        pinned_parents: &BTreeMap<VPath, Dir>,
        witness: Witness,
    ) -> Result<(), CommitError> {
        let Operation::CreateDirectory { path, .. } = operation else {
            return Ok(());
        };
        let directory = pinned_parents
            .get(path)
            .ok_or_else(|| CommitError::InternalIo {
                operation: "locate pinned created directory",
                source: io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("created directory {path} was not pinned"),
                ),
            })?;
        let stamp = stamp_dir(directory, path)?;
        if stamp.kind != witness.kind || stamp.file_id != witness.file_id {
            return Err(CommitError::InternalIo {
                operation: "created directory ownership witness mismatch",
                source: io::Error::new(io::ErrorKind::InvalidData, "ownership witness mismatch"),
            });
        }
        if !directory_owner_matches(directory, transaction, index)
            .map_err(|source| HostError::io("verify directory ownership marker", path, source))?
        {
            return Err(CommitError::InternalIo {
                operation: "directory ownership marker mismatch",
                source: io::Error::new(io::ErrorKind::InvalidData, "ownership marker mismatch"),
            });
        }
        directory
            .remove_file(DIRECTORY_OWNER_MARKER)
            .map_err(|source| HostError::io("remove directory ownership marker", path, source))?;
        sync_dir(directory).map_err(|source| {
            HostError::io("sync directory ownership marker removal", path, source)
        })?;
        Ok(())
    }

    fn verify_final(&self, plan: &PreparedPlan) -> Result<(), CommitError> {
        for (path, expected) in &plan.final_states {
            let (matches, actual) = state_matches(&self.root, path, *expected)?;
            if !matches {
                return Err(CommitError::Verification(Box::new(VerificationFailure {
                    path: path.clone(),
                    expected: *expected,
                    actual,
                })));
            }
        }
        Ok(())
    }

    fn check_fault<F: FaultInjector + ?Sized>(
        faults: &F,
        point: FaultPoint,
    ) -> Result<(), CommitError> {
        if faults.should_fail(point) {
            Err(CommitError::FaultInjected { point })
        } else {
            Ok(())
        }
    }

    fn cleanup_transaction(&self, name: &str) -> Result<(), CommitError> {
        match self.transactions.remove_dir_all(name) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(source) => {
                return Err(CommitError::InternalIo {
                    operation: "remove transaction workspace",
                    source,
                });
            }
        }
        sync_dir(&self.transactions).map_err(|source| CommitError::InternalIo {
            operation: "sync transaction cleanup",
            source,
        })
    }

    /// Recover every durable transaction workspace under this capability root.
    ///
    /// Completed-marker transactions are finalized; interrupted ones are rolled back in
    /// reverse order. Ambiguous ownership is reported and never deleted.
    ///
    /// # Errors
    ///
    /// Returns an error for corrupt journals, unsafe state transitions, or host I/O.
    #[allow(clippy::too_many_lines)]
    pub fn recover<S: TransactionStore + ?Sized>(
        &self,
        store: &S,
    ) -> Result<RecoveryReport, CommitError> {
        let _guard = WorkspaceLockGuard::exclusive(&self.coordination).map_err(|source| {
            CommitError::InternalIo {
                operation: "acquire recovery workspace lock",
                source,
            }
        })?;
        self.validate_runtime_directory()?;
        let mut names = Vec::new();
        let entries = self
            .transactions
            .entries()
            .map_err(|source| CommitError::InternalIo {
                operation: "enumerate recovery journals",
                source,
            })?;
        for entry in entries {
            let entry = entry.map_err(|source| CommitError::InternalIo {
                operation: "enumerate recovery journal",
                source,
            })?;
            if !entry
                .file_type()
                .map_err(|source| CommitError::InternalIo {
                    operation: "inspect recovery journal type",
                    source,
                })?
                .is_dir()
            {
                continue;
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| CommitError::InternalIo {
                    operation: "decode recovery transaction name",
                    source: io::Error::new(
                        io::ErrorKind::InvalidData,
                        "non-UTF-8 transaction name",
                    ),
                })?;
            names.push(name);
        }
        names.sort_unstable();
        let mut report = RecoveryReport::default();
        for name in names {
            let transaction_dir = open_real_dir(&self.transactions, &name).map_err(|source| {
                CommitError::InternalIo {
                    operation: "open recovery transaction",
                    source,
                }
            })?;
            let plan = self.read_prepared_plan(&transaction_dir)?;
            if name != plan.transaction.to_string() {
                return Err(CommitError::RecoveryConflict(RecoveryConflict {
                    transaction: plan.transaction,
                    path: None,
                    reason: "transaction directory name does not match durable plan",
                }));
            }
            let marker = has_valid_commit_marker(&transaction_dir, plan.transaction)?;
            if marker {
                if self.verify_final(&plan).is_err() {
                    report.conflicts.push(RecoveryConflict {
                        transaction: plan.transaction,
                        path: None,
                        reason: "durable commit marker exists but final state no longer verifies",
                    });
                    continue;
                }
                match store.get(plan.transaction) {
                    Ok(record) => match record.state() {
                        TransactionState::Committing => {
                            store.compare_and_transition(
                                plan.transaction,
                                TransactionState::Committing,
                                TransactionState::Committed,
                            )?;
                        }
                        TransactionState::RecoveryRequired => {
                            store.compare_and_transition(
                                plan.transaction,
                                TransactionState::RecoveryRequired,
                                TransactionState::Committed,
                            )?;
                        }
                        TransactionState::Committed => {}
                        state => {
                            report.conflicts.push(RecoveryConflict {
                                transaction: plan.transaction,
                                path: None,
                                reason: recovery_state_reason(state, true),
                            });
                            continue;
                        }
                    },
                    Err(TransactionStoreError::NotFound { .. }) => {
                        report.orphaned += 1;
                    }
                    Err(source) => return Err(source.into()),
                }
                report.finalized_commits += 1;
                drop(transaction_dir);
                self.cleanup_transaction(&name)?;
                report.cleaned += 1;
                continue;
            }

            let journal = self.read_bounded_journal(&transaction_dir)?;
            let stage =
                open_or_create_real_dir(&transaction_dir, STAGE_DIRECTORY).map_err(|source| {
                    CommitError::InternalIo {
                        operation: "open recovery staging directory",
                        source,
                    }
                })?;
            let quarantine = open_or_create_real_dir(&transaction_dir, QUARANTINE_DIRECTORY)
                .map_err(|source| CommitError::InternalIo {
                    operation: "open recovery quarantine directory",
                    source,
                })?;
            let stored_state = match store.get(plan.transaction) {
                Ok(record) => Some(record.state()),
                Err(TransactionStoreError::NotFound { .. }) => {
                    report.orphaned += 1;
                    None
                }
                Err(source) => return Err(source.into()),
            };
            if stored_state == Some(TransactionState::Committed) {
                report.conflicts.push(RecoveryConflict {
                    transaction: plan.transaction,
                    path: None,
                    reason: "store says committed but durable commit marker is missing",
                });
                continue;
            }
            if stored_state == Some(TransactionState::Committing) {
                store.compare_and_transition(
                    plan.transaction,
                    TransactionState::Committing,
                    TransactionState::RecoveryRequired,
                )?;
            }
            match self.rollback(&plan, &journal, &stage, &quarantine) {
                Ok(()) => {}
                Err(conflict) => {
                    report.conflicts.push(conflict);
                    continue;
                }
            }
            if let Some(state) = stored_state {
                let current = if state == TransactionState::Committing {
                    TransactionState::RecoveryRequired
                } else {
                    state
                };
                match current {
                    TransactionState::RecoveryRequired => {
                        store.compare_and_transition(
                            plan.transaction,
                            TransactionState::RecoveryRequired,
                            TransactionState::Failed,
                        )?;
                    }
                    TransactionState::Revalidating | TransactionState::Reserved => {
                        store.compare_and_transition(
                            plan.transaction,
                            current,
                            TransactionState::Failed,
                        )?;
                    }
                    TransactionState::Failed => {}
                    other => {
                        report.conflicts.push(RecoveryConflict {
                            transaction: plan.transaction,
                            path: None,
                            reason: recovery_state_reason(other, false),
                        });
                        continue;
                    }
                }
            }
            report.rolled_back += 1;
            drop(quarantine);
            drop(stage);
            drop(transaction_dir);
            self.cleanup_transaction(&name)?;
            report.cleaned += 1;
        }
        self.validate_runtime_directory()?;
        Ok(report)
    }

    fn read_prepared_plan(&self, transaction_dir: &Dir) -> Result<PreparedPlan, CommitError> {
        let file = open_real_file(transaction_dir, PLAN_FILE).map_err(|source| {
            CommitError::InternalIo {
                operation: "open durable commit plan",
                source,
            }
        })?;
        let mut bytes = Vec::new();
        file.take((self.config.max_plan_bytes as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|source| CommitError::InternalIo {
                operation: "read durable commit plan",
                source,
            })?;
        if bytes.len() > self.config.max_plan_bytes {
            return Err(CommitError::PlanSize {
                observed: bytes.len(),
                maximum: self.config.max_plan_bytes,
            });
        }
        PreparedPlan::decode(
            &bytes,
            self.config.max_operations,
            self.config.max_path_bytes,
        )
        .map_err(Into::into)
    }

    fn read_bounded_journal(&self, transaction_dir: &Dir) -> Result<JournalState, CommitError> {
        let metadata = transaction_dir
            .symlink_metadata(JOURNAL_FILE)
            .map_err(|source| CommitError::InternalIo {
                operation: "inspect commit journal",
                source,
            })?;
        let journal_bytes = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
        if journal_bytes > self.config.max_journal_bytes {
            return Err(CommitError::PlanSize {
                observed: journal_bytes,
                maximum: self.config.max_journal_bytes,
            });
        }
        read_journal(transaction_dir, self.config.max_journal_bytes).map_err(Into::into)
    }

    fn rollback(
        &self,
        plan: &PreparedPlan,
        journal: &JournalState,
        stage: &Dir,
        quarantine: &Dir,
    ) -> Result<(), RecoveryConflict> {
        for (index, operation) in plan.operations.iter().enumerate().rev() {
            let index = u32::try_from(index).map_err(|_| RecoveryConflict {
                transaction: plan.transaction,
                path: None,
                reason: "operation index overflow during recovery",
            })?;
            if !journal.has_intent(index) {
                continue;
            }
            let witnesses = OperationWitnesses {
                completed: journal.witness(index),
                source: journal.intent_witness(index),
                parent: journal.parent_witness(index),
            };
            let parent =
                self.open_recovery_parent(plan.transaction, operation.path(), witnesses.parent)?;
            let applied = Self::infer_applied(
                plan.transaction,
                index,
                operation,
                witnesses,
                &parent,
                stage,
                quarantine,
            )?;
            if !applied {
                continue;
            }
            Self::undo_operation(
                plan.transaction,
                index,
                operation,
                witnesses,
                &parent,
                stage,
                quarantine,
            )?;
        }
        Ok(())
    }

    fn open_recovery_parent(
        &self,
        transaction: TransactionId,
        path: &VPath,
        witness: Option<Witness>,
    ) -> Result<Dir, RecoveryConflict> {
        let parent_path = path.parent().ok_or_else(|| {
            recovery_conflict(
                transaction,
                Some(path.clone()),
                "recovery operation targets the workspace root",
            )
        })?;
        let expected = witness.ok_or_else(|| {
            recovery_conflict(
                transaction,
                Some(parent_path.clone()),
                "recovery intent lacks a parent-directory witness",
            )
        })?;
        let parent = if parent_path.is_root() {
            self.root.try_clone()
        } else {
            self.root.open_dir(relative_path(&parent_path))
        }
        .map_err(|_| {
            recovery_conflict(
                transaction,
                Some(parent_path.clone()),
                "cannot open witnessed recovery parent",
            )
        })?;
        let stamp = stamp_dir(&parent, &parent_path).map_err(|_| {
            recovery_conflict(
                transaction,
                Some(parent_path.clone()),
                "cannot inspect witnessed recovery parent",
            )
        })?;
        if stamp.kind != expected.kind || stamp.file_id != expected.file_id {
            return Err(recovery_conflict(
                transaction,
                Some(parent_path),
                "recovery parent identity changed",
            ));
        }
        Ok(parent)
    }

    #[allow(clippy::too_many_lines)]
    fn infer_applied(
        transaction: TransactionId,
        index: u32,
        operation: &Operation,
        witnesses: OperationWitnesses,
        parent: &Dir,
        stage: &Dir,
        quarantine: &Dir,
    ) -> Result<bool, RecoveryConflict> {
        if witnesses.completed.is_some() {
            return Ok(true);
        }
        let path = operation.path();
        let leaf = path
            .file_name()
            .expect("recovery operations cannot target the workspace root");
        let leaf_path = VPath::parse(leaf).expect("a VPath leaf is a valid VPath");
        match operation {
            Operation::Quarantine {
                path,
                expected,
                slot,
            } => {
                let qpath = VPath::parse(&quarantine_name(*slot)).expect("valid quarantine slot");
                let original = stamp_at(parent, &leaf_path).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot inspect incomplete quarantine operation",
                    )
                })?;
                let quarantined = stamp_at(quarantine, &qpath).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot inspect incomplete quarantine slot",
                    )
                })?;
                match (original, quarantined) {
                    (Some(_), None) => Ok(false),
                    (None, Some(_)) => relocated_state_matches(quarantine, &qpath, *expected)
                        .map_err(|_| {
                            recovery_conflict(
                                transaction,
                                Some(path.clone()),
                                "incomplete quarantine no longer matches the precondition",
                            )
                        }),
                    _ => Err(recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "incomplete quarantine has ambiguous source and destination state",
                    )),
                }
            }
            Operation::CreateDirectory { path, .. } => {
                if stamp_at(parent, &leaf_path)
                    .map_err(|_| {
                        recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "cannot inspect incomplete directory creation",
                        )
                    })?
                    .is_none()
                {
                    Ok(false)
                } else {
                    let directory = parent.open_dir(leaf).map_err(|_| {
                        recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "cannot open incomplete created directory",
                        )
                    })?;
                    directory_owner_matches(&directory, transaction, index)
                        .map_err(|_| {
                            recovery_conflict(
                                transaction,
                                Some(path.clone()),
                                "cannot verify incomplete directory ownership marker",
                            )
                        })
                        .and_then(|matches| {
                            if matches {
                                Ok(true)
                            } else {
                                Err(recovery_conflict(
                                    transaction,
                                    Some(path.clone()),
                                    "directory creation lacks a valid ownership marker",
                                ))
                            }
                        })
                }
            }
            Operation::InstallFile { path, slot, .. } => {
                if stamp_at(parent, &leaf_path)
                    .map_err(|_| {
                        recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "cannot inspect incomplete file install",
                        )
                    })?
                    .is_none()
                {
                    return Ok(false);
                }
                let stage_path = VPath::parse(&stage_name(*slot)).expect("valid stage slot");
                if same_file_pair(parent, &leaf_path, stage, &stage_path).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot prove incomplete file install ownership",
                    )
                })? {
                    Ok(true)
                } else {
                    Err(recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "incomplete file install is not the staged inode",
                    ))
                }
            }
            Operation::InstallSymlink { path, after, slot } => {
                let source_witness = witnesses.source.ok_or_else(|| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "symlink intent lacks its staged ownership witness",
                    )
                })?;
                let staged_path =
                    VPath::parse(&stage_link_name(*slot)).expect("valid staged symlink slot");
                let destination = stamp_at(parent, &leaf_path).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot inspect incomplete symlink install",
                    )
                })?;
                let staged = stamp_at(stage, &staged_path).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot inspect staged symlink ownership",
                    )
                })?;
                match (destination, staged) {
                    (None, Some(stamp)) if Witness::from(stamp) == source_witness => Ok(false),
                    (Some(stamp), None) if Witness::from(stamp) == source_witness => {
                        state_matches(parent, &leaf_path, Some(*after))
                            .map_err(|_| {
                                recovery_conflict(
                                    transaction,
                                    Some(path.clone()),
                                    "cannot verify incomplete symlink content",
                                )
                            })?
                            .0
                            .then_some(true)
                            .ok_or_else(|| {
                                recovery_conflict(
                                    transaction,
                                    Some(path.clone()),
                                    "incomplete symlink content changed",
                                )
                            })
                    }
                    (None, None) => Ok(false),
                    _ => Err(recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "incomplete symlink ownership is ambiguous",
                    )),
                }
            }
            Operation::SetDirectoryMode {
                path,
                expected,
                after_mode,
            } => {
                let Some(stamp) = stamp_at(parent, &leaf_path).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot inspect incomplete mode change",
                    )
                })?
                else {
                    return Err(recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "directory disappeared during incomplete mode change",
                    ));
                };
                let expected_id = match expected.content() {
                    Some(ContentVersion::Stamp(expected_stamp)) => Some(expected_stamp.file_id),
                    _ => None,
                };
                if expected_id.is_some_and(|id| id != stamp.file_id) {
                    return Err(recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "directory identity changed during incomplete mode change",
                    ));
                }
                if stamp.mode == *after_mode {
                    Ok(true)
                } else if stamp.mode == expected.mode() {
                    Ok(false)
                } else {
                    Err(recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "directory has neither the old nor new mode",
                    ))
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    fn undo_operation(
        transaction: TransactionId,
        index: u32,
        operation: &Operation,
        witnesses: OperationWitnesses,
        parent: &Dir,
        stage: &Dir,
        quarantine: &Dir,
    ) -> Result<(), RecoveryConflict> {
        let path = operation.path();
        let leaf = path
            .file_name()
            .expect("recovery operations cannot target the workspace root");
        let leaf_path = VPath::parse(leaf).expect("a VPath leaf is a valid VPath");
        match operation {
            Operation::Quarantine {
                path,
                expected,
                slot,
            } => {
                let qname = quarantine_name(*slot);
                let qpath = VPath::parse(&qname).expect("valid quarantine slot");
                if let Some(witness) = witnesses.completed
                    && !witness_matches(quarantine, &qpath, witness.kind, witness.file_id).map_err(
                        |_| {
                            recovery_conflict(
                                transaction,
                                Some(path.clone()),
                                "cannot verify quarantined ownership",
                            )
                        },
                    )?
                {
                    return Err(recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "quarantined node identity changed",
                    ));
                }
                if !relocated_state_matches(quarantine, &qpath, *expected).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot verify quarantined node",
                    )
                })? {
                    return Err(recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "quarantined node no longer matches its precondition",
                    ));
                }
                if stamp_at(parent, &leaf_path)
                    .map_err(|_| {
                        recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "cannot inspect rollback destination",
                        )
                    })?
                    .is_some()
                {
                    return Err(recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "rollback destination is occupied",
                    ));
                }
                quarantine.rename(&qname, parent, leaf).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot restore quarantined node",
                    )
                })?;
                sync_dir(parent).map_err(|_| {
                    recovery_conflict(transaction, Some(path.clone()), "cannot sync restored node")
                })?;
                sync_dir(quarantine).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot sync quarantine restoration",
                    )
                })?;
            }
            Operation::CreateDirectory { path, .. } => {
                if let Some(witness) = witnesses.completed {
                    if !witness_matches(parent, &leaf_path, witness.kind, witness.file_id).map_err(
                        |_| {
                            recovery_conflict(
                                transaction,
                                Some(path.clone()),
                                "cannot verify created directory ownership",
                            )
                        },
                    )? {
                        return Err(recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "created directory identity changed",
                        ));
                    }
                } else {
                    let directory = parent.open_dir(leaf).map_err(|_| {
                        recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "cannot open incomplete created directory",
                        )
                    })?;
                    let marker_present = directory_owner_matches(&directory, transaction, index)
                        .map_err(|_| {
                            recovery_conflict(
                                transaction,
                                Some(path.clone()),
                                "cannot verify created directory marker",
                            )
                        })?;
                    if !marker_present {
                        return Err(recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "created directory lacks a durable ownership proof",
                        ));
                    }
                }
                let directory = parent.open_dir(leaf).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot open created directory for rollback",
                    )
                })?;
                match directory.remove_file(DIRECTORY_OWNER_MARKER) {
                    Ok(()) => {}
                    Err(source) if source.kind() == io::ErrorKind::NotFound => {}
                    Err(_) => {
                        return Err(recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "cannot remove created-directory ownership marker",
                        ));
                    }
                }
                sync_dir(&directory).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot sync created-directory marker removal",
                    )
                })?;
                parent.remove_dir(leaf).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "created directory is not safely removable",
                    )
                })?;
                sync_dir(parent).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot sync directory rollback",
                    )
                })?;
            }
            Operation::InstallFile { path, after, slot } => {
                if let Some(witness) = witnesses.completed {
                    if !witness_matches(parent, &leaf_path, witness.kind, witness.file_id).map_err(
                        |_| {
                            recovery_conflict(
                                transaction,
                                Some(path.clone()),
                                "cannot verify installed node ownership",
                            )
                        },
                    )? {
                        return Err(recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "installed node identity changed",
                        ));
                    }
                } else {
                    let stage_path = VPath::parse(&stage_name(*slot)).expect("valid stage slot");
                    if !same_file_pair(parent, &leaf_path, stage, &stage_path).map_err(|_| {
                        recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "cannot verify incomplete installed-file ownership",
                        )
                    })? {
                        return Err(recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "incomplete installed file is not the staged inode",
                        ));
                    }
                }
                let (matches, _) =
                    state_matches(parent, &leaf_path, Some(*after)).map_err(|_| {
                        recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "cannot verify installed node content",
                        )
                    })?;
                if !matches {
                    return Err(recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "installed node content changed before rollback",
                    ));
                }
                parent.remove_file(leaf).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot remove installed node",
                    )
                })?;
                sync_dir(parent).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot sync installed-node rollback",
                    )
                })?;
            }
            Operation::InstallSymlink { path, after, .. } => {
                let witness = witnesses.completed.or(witnesses.source).ok_or_else(|| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "installed symlink lacks a durable ownership witness",
                    )
                })?;
                if !witness_matches(parent, &leaf_path, witness.kind, witness.file_id).map_err(
                    |_| {
                        recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "cannot verify installed symlink ownership",
                        )
                    },
                )? {
                    return Err(recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "installed symlink identity changed",
                    ));
                }
                let (matches, _) =
                    state_matches(parent, &leaf_path, Some(*after)).map_err(|_| {
                        recovery_conflict(
                            transaction,
                            Some(path.clone()),
                            "cannot verify installed symlink content",
                        )
                    })?;
                if !matches {
                    return Err(recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "installed symlink changed before rollback",
                    ));
                }
                parent.remove_file(leaf).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot remove installed symlink",
                    )
                })?;
                sync_dir(parent).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot sync installed-symlink rollback",
                    )
                })?;
            }
            Operation::SetDirectoryMode { path, expected, .. } => {
                if let Some(witness) = witnesses.completed
                    && !witness_matches(parent, &leaf_path, witness.kind, witness.file_id).map_err(
                        |_| {
                            recovery_conflict(
                                transaction,
                                Some(path.clone()),
                                "cannot verify mode-change ownership",
                            )
                        },
                    )?
                {
                    return Err(recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "mode-changed directory identity changed",
                    ));
                }
                let directory = parent.open_dir(leaf).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot open directory for mode rollback",
                    )
                })?;
                set_dir_mode(&directory, expected.mode()).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot restore directory mode",
                    )
                })?;
                sync_dir(&directory).map_err(|_| {
                    recovery_conflict(
                        transaction,
                        Some(path.clone()),
                        "cannot sync restored directory mode",
                    )
                })?;
            }
        }
        Ok(())
    }
}

fn same_file_pair(
    left_root: &Dir,
    left: &VPath,
    right_root: &Dir,
    right: &VPath,
) -> Result<bool, HostError> {
    let left = stamp_at(left_root, left)?;
    let right = stamp_at(right_root, right)?;
    Ok(
        matches!((left, right), (Some(left), Some(right)) if left.kind == right.kind && left.file_id == right.file_id),
    )
}

fn directory_owner_payload(transaction: TransactionId, index: u32) -> Vec<u8> {
    let mut payload = Vec::with_capacity(44);
    payload.extend_from_slice(DIRECTORY_OWNER_MAGIC);
    payload.extend_from_slice(transaction.as_bytes());
    payload.extend_from_slice(&index.to_le_bytes());
    payload
}

fn directory_owner_matches(
    directory: &Dir,
    transaction: TransactionId,
    index: u32,
) -> Result<bool, io::Error> {
    let file = match directory.open(DIRECTORY_OWNER_MARKER) {
        Ok(file) => file,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(source),
    };
    let mut bytes = Vec::new();
    file.take(45).read_to_end(&mut bytes)?;
    Ok(bytes == directory_owner_payload(transaction, index))
}

fn recovery_conflict(
    transaction: TransactionId,
    path: Option<VPath>,
    reason: &'static str,
) -> RecoveryConflict {
    RecoveryConflict {
        transaction,
        path,
        reason,
    }
}

#[allow(clippy::match_same_arms)]
fn recovery_state_reason(state: TransactionState, marker: bool) -> &'static str {
    if marker {
        match state {
            TransactionState::Created => "commit marker exists while transaction is Created",
            TransactionState::Running => "commit marker exists while transaction is Running",
            TransactionState::VirtualComplete => {
                "commit marker exists while transaction is VirtualComplete"
            }
            TransactionState::Denied => "commit marker exists while transaction is Denied",
            TransactionState::AutoApproved => {
                "commit marker exists while transaction is AutoApproved"
            }
            TransactionState::PendingApproval => {
                "commit marker exists while transaction is PendingApproval"
            }
            TransactionState::Approved => "commit marker exists while transaction is Approved",
            TransactionState::Reserved => "commit marker exists while transaction is Reserved",
            TransactionState::Revalidating => {
                "commit marker exists while transaction is Revalidating"
            }
            TransactionState::Stale => "commit marker exists while transaction is Stale",
            TransactionState::Expired => "commit marker exists while transaction is Expired",
            TransactionState::Failed => "commit marker exists while transaction is Failed",
            TransactionState::Committing
            | TransactionState::Committed
            | TransactionState::RecoveryRequired => "valid marker recovery state",
            _ => "commit marker exists in an unknown transaction state",
        }
    } else {
        match state {
            TransactionState::Created => "recovery journal exists while transaction is Created",
            TransactionState::Running => "recovery journal exists while transaction is Running",
            TransactionState::VirtualComplete => {
                "recovery journal exists while transaction is VirtualComplete"
            }
            TransactionState::Denied => "recovery journal exists while transaction is Denied",
            TransactionState::AutoApproved => {
                "recovery journal exists while transaction is AutoApproved"
            }
            TransactionState::PendingApproval => {
                "recovery journal exists while transaction is PendingApproval"
            }
            TransactionState::Approved => "recovery journal exists while transaction is Approved",
            TransactionState::Stale => "recovery journal exists while transaction is Stale",
            TransactionState::Expired => "recovery journal exists while transaction is Expired",
            TransactionState::Committed => "committed transaction has no durable marker",
            TransactionState::Reserved
            | TransactionState::Revalidating
            | TransactionState::Committing
            | TransactionState::RecoveryRequired
            | TransactionState::Failed => "valid rollback recovery state",
            _ => "recovery journal exists in an unknown transaction state",
        }
    }
}

fn parse_transaction_name(name: &str) -> Option<TransactionId> {
    if name.len() != 64 {
        return None;
    }
    let mut bytes = [0_u8; 32];
    for (index, chunk) in name.as_bytes().chunks_exact(2).enumerate() {
        let high = decode_hex(chunk[0])?;
        let low = decode_hex(chunk[1])?;
        bytes[index] = (high << 4) | low;
    }
    Some(TransactionId::from_bytes(bytes))
}

const fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
