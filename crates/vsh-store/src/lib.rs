//! Immutable blob storage plus transaction records and validated state changes.
//!
//! [`FileTransactionStore`] provides checksummed append-log durability, bounded
//! two-slot compaction, and standard-library cross-process file locks without adding
//! a database dependency. The process-local [`MemoryTransactionStore`] remains
//! available for focused tests.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use cap_std::fs::{Dir, OpenOptions};

use vsh_types::{
    ApprovalBinding, ApprovalId, BlobId, PrincipalId, SnapshotId, TransactionId, TransactionState,
    TransitionError,
};

mod directory;
mod persistent;

pub use directory::{DataDirectory, DataDirectoryError};
pub use persistent::{FileStoreConfig, FileTransactionStore};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Filesystem-backed immutable content-addressed blob storage.
///
/// Blobs are written to a temporary file in their final shard, synchronized, and then
/// atomically renamed. Every read re-hashes the bytes before returning them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlobStore {
    blobs_dir: PathBuf,
    directory: DataDirectory,
}

impl BlobStore {
    /// Open or create a blob store below `data_dir`.
    ///
    /// # Errors
    ///
    /// Returns [`BlobStoreError::Io`] if the store directory cannot be created.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, BlobStoreError> {
        let data_dir = data_dir.as_ref();
        let directory = DataDirectory::open_trusted(data_dir).map_err(|source| {
            BlobStoreError::io("open data directory", data_dir, io::Error::other(source))
        })?;
        Self::open_in(&directory)
    }

    /// Open or create a blob store below a pinned data-directory capability.
    ///
    /// # Errors
    ///
    /// Returns [`BlobStoreError::Io`] if the real `blobs` directory cannot be
    /// created, pinned, or synchronized.
    pub fn open_in(data_directory: &DataDirectory) -> Result<Self, BlobStoreError> {
        let directory = data_directory.open_real_child("blobs").map_err(|source| {
            BlobStoreError::io(
                "open capability-rooted blob store",
                &data_directory.path().join("blobs"),
                source,
            )
        })?;
        directory::sync_directory(data_directory.directory()).map_err(|source| {
            BlobStoreError::io("sync blob-store parent", data_directory.path(), source)
        })?;
        Ok(Self {
            blobs_dir: directory.path().to_path_buf(),
            directory,
        })
    }

    /// Return the directory containing the content-addressed shards.
    #[must_use]
    pub fn blobs_dir(&self) -> &Path {
        &self.blobs_dir
    }

    /// Store bytes exactly once and return their BLAKE3 identity.
    ///
    /// # Errors
    ///
    /// Returns an error for I/O failures or if an existing blob fails hash verification.
    pub fn put(&self, bytes: &[u8]) -> Result<BlobId, BlobStoreError> {
        let id = BlobId::digest(bytes);
        let (hex, shard_path, target_path) = self.location_for(id);
        let shard_name = &hex[..2];
        let target_name = &hex[2..];
        let shard = self
            .directory
            .open_real_child(shard_name)
            .map_err(|source| BlobStoreError::io("open blob shard", &shard_path, source))?;
        if entry_exists(shard.directory(), target_name)
            .map_err(|source| BlobStoreError::io("inspect immutable blob", &target_path, source))?
        {
            Self::verify_existing(id, shard.directory(), target_name, &target_path)?;
            return Ok(id);
        }

        let (mut file, temporary_name, temporary_path) =
            Self::create_temporary(shard.directory(), &shard_path)?;
        if let Err(source) = file.write_all(bytes) {
            drop(file);
            let _ = shard.directory().remove_file(&temporary_name);
            return Err(BlobStoreError::io(
                "write temporary blob",
                &temporary_path,
                source,
            ));
        }
        if let Err(source) = file.sync_all() {
            drop(file);
            let _ = shard.directory().remove_file(&temporary_name);
            return Err(BlobStoreError::io(
                "sync temporary blob",
                &temporary_path,
                source,
            ));
        }
        drop(file);

        match shard
            .directory()
            .rename(&temporary_name, shard.directory(), target_name)
        {
            Ok(()) => {}
            Err(_source)
                if entry_exists(shard.directory(), target_name).map_err(|source| {
                    BlobStoreError::io("inspect raced immutable blob", &target_path, source)
                })? =>
            {
                let _ = shard.directory().remove_file(&temporary_name);
                Self::verify_existing(id, shard.directory(), target_name, &target_path)?;
                return Ok(id);
            }
            Err(source) => {
                let _ = shard.directory().remove_file(&temporary_name);
                return Err(BlobStoreError::io(
                    "install immutable blob",
                    &target_path,
                    source,
                ));
            }
        }

        directory::sync_directory(shard.directory())
            .map_err(|source| BlobStoreError::io("sync blob shard", &shard_path, source))?;
        Self::verify_existing(id, shard.directory(), target_name, &target_path)?;
        Ok(id)
    }

    /// Load and hash-verify one immutable blob.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the blob is unavailable and [`BlobStoreError::Corrupt`]
    /// when its bytes no longer match the requested identity.
    pub fn get(&self, id: BlobId) -> Result<Vec<u8>, BlobStoreError> {
        self.get_bounded(id, usize::MAX)
    }

    /// Load and hash-verify one immutable blob without allocating beyond `maximum`.
    ///
    /// # Errors
    ///
    /// Returns [`BlobStoreError::SizeLimit`] before reading when metadata already
    /// exceeds the bound, or if a concurrently enlarged file crosses it while read.
    pub fn get_bounded(&self, id: BlobId, maximum: usize) -> Result<Vec<u8>, BlobStoreError> {
        let (hex, _shard_path, path) = self.location_for(id);
        let shard_name = &hex[..2];
        let target_name = &hex[2..];
        let shard = self
            .directory
            .directory()
            .open_dir(shard_name)
            .map_err(|source| BlobStoreError::io("open blob shard", &path, source))?;
        let mut options = OpenOptions::new();
        options.read(true);
        let mut file = directory::open_real_file(&shard, target_name, &options)
            .map_err(|source| BlobStoreError::io("open immutable blob", &path, source))?;
        let declared = usize::try_from(
            file.metadata()
                .map_err(|source| BlobStoreError::io("inspect immutable blob", &path, source))?
                .len(),
        )
        .unwrap_or(usize::MAX);
        if declared > maximum {
            return Err(BlobStoreError::SizeLimit {
                path,
                observed: declared,
                maximum,
            });
        }
        let mut bytes = Vec::with_capacity(declared);
        Read::by_ref(&mut file)
            .take(u64::try_from(maximum).unwrap_or(u64::MAX).saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|source| BlobStoreError::io("read immutable blob", &path, source))?;
        if bytes.len() > maximum {
            return Err(BlobStoreError::SizeLimit {
                path,
                observed: bytes.len(),
                maximum,
            });
        }
        let actual = BlobId::digest(&bytes);
        if actual != id {
            return Err(BlobStoreError::Corrupt {
                path,
                expected: id,
                actual,
            });
        }
        Ok(bytes)
    }

    /// Return whether a verified blob exists.
    ///
    /// # Errors
    ///
    /// Returns an error when a present blob cannot be read or fails verification.
    pub fn contains(&self, id: BlobId) -> Result<bool, BlobStoreError> {
        let (hex, _shard_path, path) = self.location_for(id);
        let shard_name = &hex[..2];
        let target_name = &hex[2..];
        let shard = match self.directory.directory().open_dir(shard_name) {
            Ok(shard) => shard,
            Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(source) => return Err(BlobStoreError::io("open blob shard", &path, source)),
        };
        if !entry_exists(&shard, target_name)
            .map_err(|source| BlobStoreError::io("inspect immutable blob", &path, source))?
        {
            return Ok(false);
        }
        Self::verify_existing(id, &shard, target_name, &path)?;
        Ok(true)
    }

    fn create_temporary(
        shard: &Dir,
        shard_path: &Path,
    ) -> Result<(File, String, PathBuf), BlobStoreError> {
        for _ in 0..1024 {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = format!(".vsh-blob-{}-{sequence}.tmp", std::process::id());
            let path = shard_path.join(&name);
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            match directory::open_real_file(shard, &name, &options) {
                Ok(file) => return Ok((file, name, path)),
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(BlobStoreError::io("create temporary blob", &path, source));
                }
            }
        }
        Err(BlobStoreError::TemporaryNameExhausted {
            directory: shard_path.to_owned(),
        })
    }

    fn verify_existing(
        id: BlobId,
        shard: &Dir,
        name: &str,
        path: &Path,
    ) -> Result<(), BlobStoreError> {
        let mut options = OpenOptions::new();
        options.read(true);
        let mut file = directory::open_real_file(shard, name, &options)
            .map_err(|source| BlobStoreError::io("verify immutable blob", path, source))?;
        let actual = hash_file(&mut file)
            .map_err(|source| BlobStoreError::io("verify immutable blob", path, source))?;
        if actual != id {
            return Err(BlobStoreError::Corrupt {
                path: path.to_owned(),
                expected: id,
                actual,
            });
        }
        Ok(())
    }

    #[cfg(test)]
    fn path_for(&self, id: BlobId) -> PathBuf {
        self.location_for(id).2
    }

    fn location_for(&self, id: BlobId) -> (String, PathBuf, PathBuf) {
        let hex = id.to_string();
        let shard = self.blobs_dir.join(&hex[..2]);
        let target = shard.join(&hex[2..]);
        (hex, shard, target)
    }
}

fn entry_exists(directory: &Dir, name: &str) -> io::Result<bool> {
    match directory.symlink_metadata(name) {
        Ok(metadata) if metadata.is_file() && !metadata.is_symlink() => Ok(true),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "immutable blob path is not a real file",
        )),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(source),
    }
}

fn hash_file(file: &mut File) -> io::Result<BlobId> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(BlobId::from_bytes(*hasher.finalize().as_bytes()))
}

/// A blob-store operation failed or immutable content did not verify.
#[derive(Debug)]
#[non_exhaustive]
pub enum BlobStoreError {
    /// A filesystem operation failed.
    Io {
        /// Short stable operation description.
        operation: &'static str,
        /// Exact path involved in the failure.
        path: PathBuf,
        /// Underlying host error.
        source: io::Error,
    },
    /// Stored bytes no longer match their content address.
    Corrupt {
        /// Corrupt blob path.
        path: PathBuf,
        /// Identity requested by the caller.
        expected: BlobId,
        /// Identity computed from the stored bytes.
        actual: BlobId,
    },
    /// A blob exceeded a caller-provided allocation bound.
    SizeLimit {
        /// Oversized blob path.
        path: PathBuf,
        /// Observed bytes.
        observed: usize,
        /// Maximum accepted bytes.
        maximum: usize,
    },
    /// Repeated unique temporary names collided unexpectedly.
    TemporaryNameExhausted {
        /// Shard in which allocation failed.
        directory: PathBuf,
    },
}

impl BlobStoreError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.to_owned(),
            source,
        }
    }
}

impl fmt::Display for BlobStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} at {}: {source}", path.display()),
            Self::Corrupt {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "blob corruption at {}: expected {expected}, got {actual}",
                path.display()
            ),
            Self::SizeLimit {
                path,
                observed,
                maximum,
            } => write!(
                formatter,
                "blob at {} is {observed} bytes; maximum is {maximum}",
                path.display()
            ),
            Self::TemporaryNameExhausted { directory } => write!(
                formatter,
                "could not allocate a unique temporary blob in {}",
                directory.display()
            ),
        }
    }
}

impl Error for BlobStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Corrupt { .. } | Self::SizeLimit { .. } | Self::TemporaryNameExhausted { .. } => {
                None
            }
        }
    }
}

/// The storage-facing identity and state of a transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransactionRecord {
    id: TransactionId,
    base_snapshot: SnapshotId,
    state: TransactionState,
    artifact: Option<BlobId>,
    approval: Option<ApprovalGrant>,
}

impl TransactionRecord {
    /// Create a transaction record in the only valid initial state.
    #[must_use]
    pub const fn new(id: TransactionId, base_snapshot: SnapshotId) -> Self {
        Self {
            id,
            base_snapshot,
            state: TransactionState::Created,
            artifact: None,
            approval: None,
        }
    }

    /// Bind an immutable content-addressed transaction artifact before persistence.
    #[must_use]
    pub const fn with_artifact(mut self, artifact: BlobId) -> Self {
        self.artifact = Some(artifact);
        self
    }

    /// Return the transaction identity.
    #[must_use]
    pub const fn id(&self) -> TransactionId {
        self.id
    }

    /// Return the immutable base snapshot identity.
    #[must_use]
    pub const fn base_snapshot(&self) -> SnapshotId {
        self.base_snapshot
    }

    /// Return the current persisted state.
    #[must_use]
    pub const fn state(&self) -> TransactionState {
        self.state
    }

    /// Return the immutable transaction artifact identity, when one is retained.
    #[must_use]
    pub const fn artifact(&self) -> Option<BlobId> {
        self.artifact
    }

    /// Return the exact independent approval grant, when one exists.
    #[must_use]
    pub const fn approval(&self) -> Option<ApprovalGrant> {
        self.approval
    }

    /// Apply a valid state transition or leave the record unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`TransitionError`] when `next` is not reachable from the current state.
    pub fn transition(&mut self, next: TransactionState) -> Result<(), TransitionError> {
        if !self.state.can_transition_to(next) {
            return Err(TransitionError {
                from: self.state,
                to: next,
            });
        }
        self.state = next;
        Ok(())
    }
}

/// An independent approval bound to one exact transaction and expiry window.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApprovalGrant {
    binding: ApprovalBinding,
    id: ApprovalId,
}

impl ApprovalGrant {
    /// Construct a bounded approval grant.
    ///
    /// # Errors
    ///
    /// Returns [`ApprovalGrantError`] unless expiry is strictly after issuance.
    pub fn new(
        transaction: TransactionId,
        principal: PrincipalId,
        issued_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Result<Self, ApprovalGrantError> {
        if expires_at_unix_ms <= issued_at_unix_ms {
            return Err(ApprovalGrantError::InvalidWindow {
                issued_at_unix_ms,
                expires_at_unix_ms,
            });
        }
        let binding = ApprovalBinding {
            transaction,
            principal,
            issued_at_unix_ms,
            expires_at_unix_ms,
        };
        Ok(Self {
            binding,
            id: binding.approval_id(),
        })
    }

    /// Return the immutable approval identity.
    #[must_use]
    pub const fn id(self) -> ApprovalId {
        self.id
    }

    /// Return the exact transaction approved.
    #[must_use]
    pub const fn transaction(self) -> TransactionId {
        self.binding.transaction
    }

    /// Return the opaque approving principal.
    #[must_use]
    pub const fn principal(self) -> PrincipalId {
        self.binding.principal
    }

    /// Return issuance time in Unix milliseconds.
    #[must_use]
    pub const fn issued_at_unix_ms(self) -> u64 {
        self.binding.issued_at_unix_ms
    }

    /// Return exclusive expiry time in Unix milliseconds.
    #[must_use]
    pub const fn expires_at_unix_ms(self) -> u64 {
        self.binding.expires_at_unix_ms
    }

    /// Return whether the grant is expired at `now_unix_ms`.
    #[must_use]
    pub const fn is_expired_at(self, now_unix_ms: u64) -> bool {
        now_unix_ms >= self.binding.expires_at_unix_ms
    }
}

/// Invalid approval grant input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApprovalGrantError {
    /// Expiry did not follow issuance.
    InvalidWindow {
        /// Supplied issuance time.
        issued_at_unix_ms: u64,
        /// Supplied expiry time.
        expires_at_unix_ms: u64,
    },
}

impl fmt::Display for ApprovalGrantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWindow {
                issued_at_unix_ms,
                expires_at_unix_ms,
            } => write!(
                formatter,
                "approval expiry {expires_at_unix_ms} must follow issuance {issued_at_unix_ms}"
            ),
        }
    }
}

impl Error for ApprovalGrantError {}

/// Non-cloneable proof that one transaction won the atomic commit reservation.
#[derive(Debug, Eq, PartialEq)]
pub struct CommitReservation {
    transaction: TransactionId,
    base_snapshot: SnapshotId,
}

impl CommitReservation {
    /// Return the reserved transaction identity.
    #[must_use]
    pub const fn transaction(&self) -> TransactionId {
        self.transaction
    }

    /// Return the immutable snapshot the transaction was simulated against.
    #[must_use]
    pub const fn base_snapshot(&self) -> SnapshotId {
        self.base_snapshot
    }
}

/// Atomic transaction-state operations required by the runtime and committer.
pub trait TransactionStore: Send + Sync {
    /// Insert one record whose state was reached through validated in-memory edges.
    ///
    /// # Errors
    ///
    /// Returns [`TransactionStoreError::Duplicate`] when the ID already exists.
    fn create(&self, record: TransactionRecord) -> Result<(), TransactionStoreError>;

    /// Load one immutable record snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`TransactionStoreError::NotFound`] for an unknown ID.
    fn get(&self, id: TransactionId) -> Result<TransactionRecord, TransactionStoreError>;

    /// Compare exact state and perform one valid transition atomically.
    ///
    /// # Errors
    ///
    /// Returns a state conflict or transition error without modifying the record.
    fn compare_and_transition(
        &self,
        id: TransactionId,
        expected: TransactionState,
        next: TransactionState,
    ) -> Result<TransactionRecord, TransactionStoreError>;

    /// Bind an independent grant and move `PendingApproval` to `Approved` atomically.
    ///
    /// # Errors
    ///
    /// Returns an error for an ID mismatch, missing record, or wrong current state.
    fn approve(
        &self,
        id: TransactionId,
        grant: ApprovalGrant,
    ) -> Result<TransactionRecord, TransactionStoreError>;

    /// Atomically consume `AutoApproved` or unexpired `Approved` into `Reserved`.
    ///
    /// # Errors
    ///
    /// Returns a state conflict, missing-approval error, or expiry error. An expired
    /// record is atomically moved to `Expired`.
    fn reserve(
        &self,
        id: TransactionId,
        now_unix_ms: u64,
    ) -> Result<CommitReservation, TransactionStoreError>;
}

/// Process-local reference backend for state-machine and concurrency correctness.
///
/// The lock covers only short record operations; virtual execution never occurs while
/// it is held. Production runtimes use [`FileTransactionStore`] for crash durability.
#[derive(Clone, Debug, Default)]
pub struct MemoryTransactionStore {
    records: Arc<Mutex<BTreeMap<TransactionId, TransactionRecord>>>,
}

impl MemoryTransactionStore {
    fn lock(
        &self,
    ) -> Result<MutexGuard<'_, BTreeMap<TransactionId, TransactionRecord>>, TransactionStoreError>
    {
        self.records
            .lock()
            .map_err(|_| TransactionStoreError::Poisoned)
    }
}

impl TransactionStore for MemoryTransactionStore {
    fn create(&self, record: TransactionRecord) -> Result<(), TransactionStoreError> {
        let mut records = self.lock()?;
        if records.contains_key(&record.id()) {
            return Err(TransactionStoreError::Duplicate { id: record.id() });
        }
        records.insert(record.id(), record);
        Ok(())
    }

    fn get(&self, id: TransactionId) -> Result<TransactionRecord, TransactionStoreError> {
        self.lock()?
            .get(&id)
            .cloned()
            .ok_or(TransactionStoreError::NotFound { id })
    }

    fn compare_and_transition(
        &self,
        id: TransactionId,
        expected: TransactionState,
        next: TransactionState,
    ) -> Result<TransactionRecord, TransactionStoreError> {
        let mut records = self.lock()?;
        let record = records
            .get_mut(&id)
            .ok_or(TransactionStoreError::NotFound { id })?;
        if record.state() != expected {
            return Err(TransactionStoreError::StateConflict {
                id,
                expected,
                actual: record.state(),
            });
        }
        record
            .transition(next)
            .map_err(TransactionStoreError::Transition)?;
        Ok(record.clone())
    }

    fn approve(
        &self,
        id: TransactionId,
        grant: ApprovalGrant,
    ) -> Result<TransactionRecord, TransactionStoreError> {
        if grant.transaction() != id {
            return Err(TransactionStoreError::ApprovalBindingMismatch {
                requested: id,
                bound: grant.transaction(),
            });
        }
        let mut records = self.lock()?;
        let record = records
            .get_mut(&id)
            .ok_or(TransactionStoreError::NotFound { id })?;
        if record.state() != TransactionState::PendingApproval {
            return Err(TransactionStoreError::StateConflict {
                id,
                expected: TransactionState::PendingApproval,
                actual: record.state(),
            });
        }
        record
            .transition(TransactionState::Approved)
            .map_err(TransactionStoreError::Transition)?;
        record.approval = Some(grant);
        Ok(record.clone())
    }

    fn reserve(
        &self,
        id: TransactionId,
        now_unix_ms: u64,
    ) -> Result<CommitReservation, TransactionStoreError> {
        let mut records = self.lock()?;
        let record = records
            .get_mut(&id)
            .ok_or(TransactionStoreError::NotFound { id })?;
        match record.state() {
            TransactionState::AutoApproved => {}
            TransactionState::Approved => {
                let grant = record
                    .approval()
                    .ok_or(TransactionStoreError::MissingApproval { id })?;
                if grant.is_expired_at(now_unix_ms) {
                    record
                        .transition(TransactionState::Expired)
                        .map_err(TransactionStoreError::Transition)?;
                    return Err(TransactionStoreError::ApprovalExpired {
                        id,
                        expired_at_unix_ms: grant.expires_at_unix_ms(),
                        observed_at_unix_ms: now_unix_ms,
                    });
                }
            }
            actual => {
                return Err(TransactionStoreError::NotReservable { id, actual });
            }
        }
        record
            .transition(TransactionState::Reserved)
            .map_err(TransactionStoreError::Transition)?;
        Ok(CommitReservation {
            transaction: id,
            base_snapshot: record.base_snapshot(),
        })
    }
}

/// Atomic transaction-store failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TransactionStoreError {
    /// The transaction ID already exists.
    Duplicate {
        /// Duplicate ID.
        id: TransactionId,
    },
    /// No record has this ID.
    NotFound {
        /// Missing ID.
        id: TransactionId,
    },
    /// Compare-and-transition observed another state.
    StateConflict {
        /// Transaction ID.
        id: TransactionId,
        /// Required current state.
        expected: TransactionState,
        /// State actually observed.
        actual: TransactionState,
    },
    /// The requested state edge is invalid.
    Transition(TransitionError),
    /// A grant for another exact artifact was presented.
    ApprovalBindingMismatch {
        /// Transaction the caller attempted to approve.
        requested: TransactionId,
        /// Transaction actually covered by the grant.
        bound: TransactionId,
    },
    /// An approved state lacked its grant and therefore failed closed.
    MissingApproval {
        /// Affected transaction.
        id: TransactionId,
    },
    /// Approval expired and the record was moved to `Expired`.
    ApprovalExpired {
        /// Affected transaction.
        id: TransactionId,
        /// Exclusive grant expiry.
        expired_at_unix_ms: u64,
        /// Time supplied to reservation.
        observed_at_unix_ms: u64,
    },
    /// Current state can never win a new reservation.
    NotReservable {
        /// Affected transaction.
        id: TransactionId,
        /// State actually observed.
        actual: TransactionState,
    },
    /// Durable state-file or cross-process lock I/O failed.
    PersistentIo {
        /// Stable operation label.
        operation: &'static str,
        /// Portable operating-system error category.
        kind: io::ErrorKind,
    },
    /// Durable state bytes failed structural, checksum, or lifecycle validation.
    PersistentCorrupt {
        /// Byte offset at which validation failed.
        offset: u64,
        /// Stable corruption reason.
        reason: &'static str,
    },
    /// The append-only durable log exceeded its configured byte bound.
    PersistentLogLimit {
        /// Bytes the next durable state would require.
        observed: u64,
        /// Configured byte ceiling.
        maximum: u64,
    },
    /// Unique durable transaction count exceeded its configured bound.
    PersistentRecordLimit {
        /// Unique records the operation would retain.
        observed: usize,
        /// Configured record ceiling.
        maximum: usize,
    },
    /// Another thread panicked while mutating the in-memory reference backend.
    Poisoned,
}

impl fmt::Display for TransactionStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Duplicate { id } => write!(formatter, "duplicate transaction: {id}"),
            Self::NotFound { id } => write!(formatter, "unknown transaction: {id}"),
            Self::StateConflict {
                id,
                expected,
                actual,
            } => write!(
                formatter,
                "transaction {id} state conflict: expected {expected:?}, got {actual:?}"
            ),
            Self::Transition(source) => fmt::Display::fmt(source, formatter),
            Self::ApprovalBindingMismatch { requested, bound } => write!(
                formatter,
                "approval binding mismatch: requested {requested}, grant covers {bound}"
            ),
            Self::MissingApproval { id } => {
                write!(formatter, "transaction {id} has no bound approval grant")
            }
            Self::ApprovalExpired {
                id,
                expired_at_unix_ms,
                observed_at_unix_ms,
            } => write!(
                formatter,
                "transaction {id} approval expired at {expired_at_unix_ms}; observed {observed_at_unix_ms}"
            ),
            Self::NotReservable { id, actual } => {
                write!(
                    formatter,
                    "transaction {id} is not reservable from {actual:?}"
                )
            }
            Self::PersistentIo { operation, kind } => {
                write!(
                    formatter,
                    "persistent transaction store {operation} failed: {kind}"
                )
            }
            Self::PersistentCorrupt { offset, reason } => write!(
                formatter,
                "persistent transaction store is corrupt at byte {offset}: {reason}"
            ),
            Self::PersistentLogLimit { observed, maximum } => write!(
                formatter,
                "persistent transaction log would use {observed} bytes; maximum is {maximum}"
            ),
            Self::PersistentRecordLimit { observed, maximum } => write!(
                formatter,
                "persistent transaction store would retain {observed} records; maximum is {maximum}"
            ),
            Self::Poisoned => formatter.write_str("transaction store lock is poisoned"),
        }
    }
}

impl Error for TransactionStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Transition(source) => Some(source),
            Self::Duplicate { .. }
            | Self::NotFound { .. }
            | Self::StateConflict { .. }
            | Self::ApprovalBindingMismatch { .. }
            | Self::MissingApproval { .. }
            | Self::ApprovalExpired { .. }
            | Self::NotReservable { .. }
            | Self::PersistentIo { .. }
            | Self::PersistentCorrupt { .. }
            | Self::PersistentLogLimit { .. }
            | Self::PersistentRecordLimit { .. }
            | Self::Poisoned => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io;
    use std::sync::{Arc, Barrier};
    use std::thread;

    struct TestDirectory(PathBuf);

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_store(name: &str) -> (TestDirectory, BlobStore) {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "vsh-store-test-{}-{sequence}-{name}",
            std::process::id()
        ));
        let guard = TestDirectory(root.clone());
        let store = BlobStore::open(root).unwrap();
        (guard, store)
    }

    #[test]
    fn blobs_are_content_addressed_deduplicated_and_verified() {
        let (_guard, store) = test_store("round-trip");
        let bytes = b"immutable vsh blob";

        let first = store.put(bytes).unwrap();
        let second = store.put(bytes).unwrap();

        assert_eq!(first, second);
        assert_eq!(store.get(first).unwrap(), bytes);
        assert!(store.contains(first).unwrap());
        assert!(!store.contains(BlobId::from_bytes([0xff; 32])).unwrap());
    }

    #[test]
    fn corrupt_blob_is_never_returned() {
        let (_guard, store) = test_store("corrupt");
        let id = store.put(b"expected").unwrap();
        let path = store.path_for(id);
        fs::write(&path, b"tampered").unwrap();

        let error = store.get(id).unwrap_err();
        assert!(matches!(
            error,
            BlobStoreError::Corrupt {
                expected,
                actual: _,
                path: _
            } if expected == id
        ));
        assert!(store.put(b"expected").is_err());
    }

    #[test]
    fn bounded_blob_read_rejects_oversize_before_returning_bytes() {
        let (_guard, store) = test_store("bounded");
        let id = store.put(b"12345678").unwrap();

        let error = store.get_bounded(id, 7).unwrap_err();
        assert!(matches!(
            error,
            BlobStoreError::SizeLimit {
                observed: 8,
                maximum: 7,
                path: _
            }
        ));
        assert_eq!(store.get_bounded(id, 8).unwrap(), b"12345678");
    }

    #[test]
    fn record_preserves_identity_and_validates_transitions() {
        let id = TransactionId::from_bytes([1; 32]);
        let snapshot = SnapshotId::from_bytes([2; 32]);
        let mut record = TransactionRecord::new(id, snapshot);

        assert_eq!(record.id(), id);
        assert_eq!(record.base_snapshot(), snapshot);
        assert_eq!(record.state(), TransactionState::Created);

        record.transition(TransactionState::Running).unwrap();
        assert_eq!(record.state(), TransactionState::Running);

        let error = record.transition(TransactionState::Committed).unwrap_err();
        assert_eq!(
            error,
            TransitionError {
                from: TransactionState::Running,
                to: TransactionState::Committed,
            }
        );
        assert_eq!(record.state(), TransactionState::Running);
    }

    fn policy_complete_store(id: TransactionId, state: TransactionState) -> MemoryTransactionStore {
        let store = MemoryTransactionStore::default();
        store
            .create(TransactionRecord::new(id, SnapshotId::from_bytes([2; 32])))
            .unwrap();
        store
            .compare_and_transition(id, TransactionState::Created, TransactionState::Running)
            .unwrap();
        store
            .compare_and_transition(
                id,
                TransactionState::Running,
                TransactionState::VirtualComplete,
            )
            .unwrap();
        store
            .compare_and_transition(id, TransactionState::VirtualComplete, state)
            .unwrap();
        store
    }

    #[test]
    fn approval_is_bound_to_exact_transaction_and_expires_closed() {
        let id = TransactionId::from_bytes([3; 32]);
        let other = TransactionId::from_bytes([4; 32]);
        let store = policy_complete_store(id, TransactionState::PendingApproval);
        let principal = PrincipalId::digest_label("fresh-judge");
        let wrong_grant = ApprovalGrant::new(other, principal, 100, 200).unwrap();
        assert!(matches!(
            store.approve(id, wrong_grant),
            Err(TransactionStoreError::ApprovalBindingMismatch {
                requested,
                bound
            }) if requested == id && bound == other
        ));

        let grant = ApprovalGrant::new(id, principal, 100, 200).unwrap();
        let approval_id = grant.id();
        let approved = store.approve(id, grant).unwrap();
        assert_eq!(approved.approval().unwrap().id(), approval_id);
        assert!(matches!(
            store.reserve(id, 200),
            Err(TransactionStoreError::ApprovalExpired { .. })
        ));
        assert_eq!(store.get(id).unwrap().state(), TransactionState::Expired);
    }

    #[test]
    fn approval_window_must_be_forward_and_digest_is_deterministic() {
        let id = TransactionId::from_bytes([5; 32]);
        let principal = PrincipalId::digest_label("judge");
        assert!(ApprovalGrant::new(id, principal, 10, 10).is_err());
        let first = ApprovalGrant::new(id, principal, 10, 11).unwrap();
        let second = ApprovalGrant::new(id, principal, 10, 11).unwrap();
        assert_eq!(first.id(), second.id());
    }

    #[test]
    fn atomic_reservation_is_single_use_under_concurrency() {
        const CONTENDERS: usize = 8;
        let id = TransactionId::from_bytes([6; 32]);
        let store = policy_complete_store(id, TransactionState::AutoApproved);
        let barrier = Arc::new(Barrier::new(CONTENDERS));
        let mut threads = Vec::new();
        for _ in 0..CONTENDERS {
            let store = store.clone();
            let barrier = Arc::clone(&barrier);
            threads.push(thread::spawn(move || {
                barrier.wait();
                store.reserve(id, 0).is_ok()
            }));
        }
        let winners = threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .filter(|won| *won)
            .count();

        assert_eq!(winners, 1);
        assert_eq!(store.get(id).unwrap().state(), TransactionState::Reserved);
        assert!(matches!(
            store.reserve(id, 0),
            Err(TransactionStoreError::NotReservable {
                actual: TransactionState::Reserved,
                ..
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn blob_shard_symlink_cannot_redirect_a_write() {
        use std::os::unix::fs::symlink;

        let (_guard, store) = test_store("shard-symlink");
        let outside = std::env::temp_dir().join(format!(
            "vsh-store-outside-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&outside).unwrap();
        let id = BlobId::digest(b"cannot escape");
        let shard = &id.to_string()[..2];
        symlink(&outside, store.blobs_dir().join(shard)).unwrap();

        assert!(store.put(b"cannot escape").is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);

        fs::remove_dir(&outside).unwrap();
    }

    #[test]
    fn public_store_errors_have_stable_messages_and_sources() {
        let first = TransactionId::from_bytes([1; 32]);
        let second = TransactionId::from_bytes([2; 32]);
        let transition = TransitionError {
            from: TransactionState::Created,
            to: TransactionState::Committed,
        };
        let errors = [
            TransactionStoreError::Duplicate { id: first },
            TransactionStoreError::NotFound { id: first },
            TransactionStoreError::StateConflict {
                id: first,
                expected: TransactionState::Running,
                actual: TransactionState::Created,
            },
            TransactionStoreError::Transition(transition),
            TransactionStoreError::ApprovalBindingMismatch {
                requested: first,
                bound: second,
            },
            TransactionStoreError::MissingApproval { id: first },
            TransactionStoreError::ApprovalExpired {
                id: first,
                expired_at_unix_ms: 10,
                observed_at_unix_ms: 11,
            },
            TransactionStoreError::NotReservable {
                id: first,
                actual: TransactionState::Denied,
            },
            TransactionStoreError::PersistentIo {
                operation: "read",
                kind: io::ErrorKind::PermissionDenied,
            },
            TransactionStoreError::PersistentCorrupt {
                offset: 7,
                reason: "test",
            },
            TransactionStoreError::PersistentLogLimit {
                observed: 2,
                maximum: 1,
            },
            TransactionStoreError::PersistentRecordLimit {
                observed: 2,
                maximum: 1,
            },
            TransactionStoreError::Poisoned,
        ];
        for error in errors {
            assert!(!error.to_string().is_empty());
            assert_eq!(
                Error::source(&error).is_some(),
                matches!(error, TransactionStoreError::Transition(_))
            );
        }

        let expected = BlobId::from_bytes([3; 32]);
        let actual = BlobId::from_bytes([4; 32]);
        let blob_errors = [
            BlobStoreError::Io {
                operation: "read",
                path: PathBuf::from("blob"),
                source: io::Error::other("test"),
            },
            BlobStoreError::Corrupt {
                path: PathBuf::from("blob"),
                expected,
                actual,
            },
            BlobStoreError::SizeLimit {
                path: PathBuf::from("blob"),
                observed: 2,
                maximum: 1,
            },
            BlobStoreError::TemporaryNameExhausted {
                directory: PathBuf::from("blobs"),
            },
        ];
        for error in blob_errors {
            assert!(!error.to_string().is_empty());
            assert_eq!(
                Error::source(&error).is_some(),
                matches!(error, BlobStoreError::Io { .. })
            );
        }

        let approval =
            ApprovalGrant::new(first, PrincipalId::digest_label("test"), 2, 1).unwrap_err();
        assert_eq!(
            approval.to_string(),
            "approval expiry 1 must follow issuance 2"
        );
    }
}
