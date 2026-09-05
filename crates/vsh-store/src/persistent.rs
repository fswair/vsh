use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use cap_std::fs::{Dir, OpenOptions};
use vsh_types::{BlobId, PrincipalId, SnapshotId, TransactionId, TransactionState};

use super::{
    ApprovalGrant, CommitReservation, DataDirectory, TransactionRecord, TransactionStore,
    TransactionStoreError, directory,
};

const LOG_MAGIC: &[u8; 8] = b"VSHST001";
const CONTROL_MAGIC: &[u8; 8] = b"VSHCT001";
const LOCK_FILE: &str = "transactions.lock";
const LOG_FILE: &str = "transactions.vsh";
const ALTERNATE_LOG_FILE: &str = "transactions.alt.vsh";
const HEADER_BYTES: u64 = LOG_MAGIC.len() as u64;
const CONTROL_HEADER_BYTES: u64 = CONTROL_MAGIC.len() as u64;
const CONTROL_PAYLOAD_BYTES: usize = 8 + 1;
const CONTROL_RECORD_BYTES: usize = CONTROL_PAYLOAD_BYTES + DIGEST_BYTES;
const CONTROL_SLOT_COUNT: usize = 2;
const CONTROL_BYTES: u64 =
    CONTROL_HEADER_BYTES + (CONTROL_RECORD_BYTES * CONTROL_SLOT_COUNT) as u64;
const DIGEST_BYTES: usize = 32;
const OLD_MIN_PAYLOAD_BYTES: usize = 32 + 32 + 1 + 1;
const MIN_PAYLOAD_BYTES: usize = 32 + 32 + 1 + 1 + 1;
const ARTIFACT_PAYLOAD_BYTES: usize = 32;
const APPROVAL_PAYLOAD_BYTES: usize = 32 + 8 + 8;
const OLD_MAX_PAYLOAD_BYTES: usize = OLD_MIN_PAYLOAD_BYTES + APPROVAL_PAYLOAD_BYTES;
const MAX_PAYLOAD_BYTES: usize =
    MIN_PAYLOAD_BYTES + ARTIFACT_PAYLOAD_BYTES + APPROVAL_PAYLOAD_BYTES;

/// Hard bounds for the dependency-free compacting transaction state store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileStoreConfig {
    /// Maximum active log bytes, including framing and checksums. When the next
    /// append would cross this bound, the latest record for every transaction is
    /// rewritten to the inactive slot and atomically selected by the control file.
    pub max_log_bytes: u64,
    /// Maximum unique transactions retained by one store.
    pub max_records: usize,
}

impl Default for FileStoreConfig {
    fn default() -> Self {
        Self {
            max_log_bytes: 256 * 1024 * 1024,
            max_records: 1_000_000,
        }
    }
}

#[derive(Debug, Default)]
struct PersistentState {
    records: BTreeMap<TransactionId, TransactionRecord>,
    offset: u64,
    generation: u64,
    slot: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ControlState {
    generation: u64,
    slot: u8,
}

/// Durable cross-process transaction state using only the Rust standard library.
///
/// Each operation takes a short process-local mutex and an OS file lock, refreshes only
/// unseen append-log bytes, validates checksums and lifecycle edges, then synchronizes
/// at most one complete record. At the configured byte ceiling it compacts into the
/// inactive log and switches a fixed-size, double-buffered control record. Monty
/// execution, VFS work, and host commit operations never run while either lock is held.
#[derive(Clone, Debug)]
pub struct FileTransactionStore {
    state: Arc<Mutex<PersistentState>>,
    lock: Arc<File>,
    data_directory: DataDirectory,
    config: FileStoreConfig,
}

impl FileTransactionStore {
    /// Open or initialize a durable transaction store below `data_directory`.
    ///
    /// # Errors
    ///
    /// Returns an error for filesystem/lock failures, invalid bounds, corruption, an
    /// invalid persisted state edge, or a configured size ceiling.
    pub fn open(
        data_directory: impl AsRef<Path>,
        config: FileStoreConfig,
    ) -> Result<Self, TransactionStoreError> {
        validate_config(config)?;
        let data_directory = DataDirectory::open_trusted(data_directory).map_err(|source| {
            persistent_io("open trusted data directory", io::Error::other(source))
        })?;
        Self::open_validated_in(&data_directory, config)
    }

    /// Open or initialize a durable transaction store below a pinned capability.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid bounds, filesystem/lock failures, corruption,
    /// an invalid persisted state edge, or a configured size ceiling.
    pub fn open_in(
        data_directory: &DataDirectory,
        config: FileStoreConfig,
    ) -> Result<Self, TransactionStoreError> {
        validate_config(config)?;
        Self::open_validated_in(data_directory, config)
    }

    fn open_validated_in(
        data_directory: &DataDirectory,
        config: FileStoreConfig,
    ) -> Result<Self, TransactionStoreError> {
        let mut lock_options = OpenOptions::new();
        lock_options
            .read(true)
            .write(true)
            .create(true)
            .truncate(false);
        let lock = directory::open_real_file(data_directory.directory(), LOCK_FILE, &lock_options)
            .map_err(|source| persistent_io("open lock file", source))?;
        let guard = FileLockGuard::exclusive(&lock)?;
        let control = initialize_control(&lock)?;
        let mut log = open_log(data_directory.directory(), control.slot)?;
        initialize_header(&mut log)?;
        let mut state = PersistentState {
            records: BTreeMap::new(),
            offset: HEADER_BYTES,
            generation: control.generation,
            slot: control.slot,
        };
        refresh(&mut state, &mut log, config, control)?;
        directory::sync_directory(data_directory.directory())
            .map_err(|source| persistent_io("sync data directory", source))?;
        drop(guard);
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            lock: Arc::new(lock),
            data_directory: data_directory.clone(),
            config,
        })
    }

    /// Return the currently active state-log path for diagnostics and backup tooling.
    ///
    /// # Errors
    ///
    /// Returns an error if the cross-process lock or compact-log control journal
    /// cannot be read safely.
    pub fn active_log_path(&self) -> Result<PathBuf, TransactionStoreError> {
        let _guard = FileLockGuard::exclusive(&self.lock)?;
        let control = read_control(&self.lock)?;
        Ok(log_path_for(self.data_directory.path(), control.slot))
    }

    fn state(&self) -> Result<MutexGuard<'_, PersistentState>, TransactionStoreError> {
        self.state
            .lock()
            .map_err(|_| TransactionStoreError::Poisoned)
    }

    fn transact<R>(
        &self,
        operation: impl FnOnce(
            &BTreeMap<TransactionId, TransactionRecord>,
        ) -> Result<Mutation<R>, TransactionStoreError>,
    ) -> Result<R, TransactionStoreError> {
        let mut state = self.state()?;
        let _guard = FileLockGuard::exclusive(&self.lock)?;
        let control = read_control(&self.lock)?;
        let mut log = open_log(self.data_directory.directory(), control.slot)?;
        initialize_header(&mut log)?;
        refresh(&mut state, &mut log, self.config, control)?;
        let mutation = operation(&state.records)?;
        if let Some(record) = mutation.persist {
            if !state.records.contains_key(&record.id())
                && state.records.len() >= self.config.max_records
            {
                return Err(TransactionStoreError::PersistentRecordLimit {
                    observed: state.records.len().saturating_add(1),
                    maximum: self.config.max_records,
                });
            }
            match append_record(&mut log, &record, &mut state.offset, self.config) {
                Ok(()) => {
                    state.records.insert(record.id(), record);
                }
                Err(TransactionStoreError::PersistentLogLimit { .. }) => {
                    self.compact(&mut state, control, record)?;
                }
                Err(error) => return Err(error),
            }
        }
        mutation.result
    }

    fn compact(
        &self,
        state: &mut PersistentState,
        control: ControlState,
        record: TransactionRecord,
    ) -> Result<(), TransactionStoreError> {
        let generation =
            control
                .generation
                .checked_add(1)
                .ok_or(TransactionStoreError::PersistentCorrupt {
                    offset: CONTROL_HEADER_BYTES,
                    reason: "state-log control generation exhausted",
                })?;
        let next = ControlState {
            generation,
            slot: control.slot ^ 1,
        };
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(true);
        let mut compacted = directory::open_real_file(
            self.data_directory.directory(),
            log_name_for(next.slot),
            &options,
        )
        .map_err(|source| persistent_io("open compacted state log", source))?;
        compacted
            .write_all(LOG_MAGIC)
            .map_err(|source| persistent_io("write compacted state header", source))?;
        let mut offset = HEADER_BYTES;
        let mut wrote_record = false;
        for (id, existing) in &state.records {
            if !wrote_record && record.id() < *id {
                offset = write_record_frame(&mut compacted, &record, offset, self.config)?;
                wrote_record = true;
            }
            if *id == record.id() {
                offset = write_record_frame(&mut compacted, &record, offset, self.config)?;
                wrote_record = true;
            } else {
                offset = write_record_frame(&mut compacted, existing, offset, self.config)?;
            }
        }
        if !wrote_record {
            offset = write_record_frame(&mut compacted, &record, offset, self.config)?;
        }
        compacted
            .sync_all()
            .map_err(|source| persistent_io("sync compacted state log", source))?;
        directory::sync_directory(self.data_directory.directory())
            .map_err(|source| persistent_io("sync compacted state directory", source))?;
        append_control(&self.lock, control, next)?;

        state.records.insert(record.id(), record);
        state.offset = offset;
        state.generation = next.generation;
        state.slot = next.slot;
        Ok(())
    }
}

impl TransactionStore for FileTransactionStore {
    fn create(&self, record: TransactionRecord) -> Result<(), TransactionStoreError> {
        validate_record(&record, 0)?;
        self.transact(|records| {
            if records.contains_key(&record.id()) {
                return Err(TransactionStoreError::Duplicate { id: record.id() });
            }
            Ok(Mutation::persist(record, ()))
        })
    }

    fn get(&self, id: TransactionId) -> Result<TransactionRecord, TransactionStoreError> {
        self.transact(|records| {
            records
                .get(&id)
                .cloned()
                .map(Mutation::return_value)
                .ok_or(TransactionStoreError::NotFound { id })
        })
    }

    fn compare_and_transition(
        &self,
        id: TransactionId,
        expected: TransactionState,
        next: TransactionState,
    ) -> Result<TransactionRecord, TransactionStoreError> {
        self.transact(|records| {
            let mut record = records
                .get(&id)
                .cloned()
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
            Ok(Mutation::persist(record.clone(), record))
        })
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
        self.transact(|records| {
            let mut record = records
                .get(&id)
                .cloned()
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
            Ok(Mutation::persist(record.clone(), record))
        })
    }

    fn reserve(
        &self,
        id: TransactionId,
        now_unix_ms: u64,
    ) -> Result<CommitReservation, TransactionStoreError> {
        self.transact(|records| {
            let mut record = records
                .get(&id)
                .cloned()
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
                        return Ok(Mutation::persist_error(
                            record,
                            TransactionStoreError::ApprovalExpired {
                                id,
                                expired_at_unix_ms: grant.expires_at_unix_ms(),
                                observed_at_unix_ms: now_unix_ms,
                            },
                        ));
                    }
                }
                actual => {
                    return Err(TransactionStoreError::NotReservable { id, actual });
                }
            }
            record
                .transition(TransactionState::Reserved)
                .map_err(TransactionStoreError::Transition)?;
            let reservation = CommitReservation {
                transaction: id,
                base_snapshot: record.base_snapshot(),
            };
            Ok(Mutation::persist(record, reservation))
        })
    }
}

struct Mutation<R> {
    persist: Option<TransactionRecord>,
    result: Result<R, TransactionStoreError>,
}

impl<R> Mutation<R> {
    fn return_value(value: R) -> Self {
        Self {
            persist: None,
            result: Ok(value),
        }
    }

    fn persist(record: TransactionRecord, value: R) -> Self {
        Self {
            persist: Some(record),
            result: Ok(value),
        }
    }

    fn persist_error(record: TransactionRecord, error: TransactionStoreError) -> Self {
        Self {
            persist: Some(record),
            result: Err(error),
        }
    }
}

struct FileLockGuard<'a>(&'a File);

impl<'a> FileLockGuard<'a> {
    fn exclusive(file: &'a File) -> Result<Self, TransactionStoreError> {
        File::lock(file).map_err(|source| persistent_io("acquire lock", source))?;
        Ok(Self(file))
    }
}

impl Drop for FileLockGuard<'_> {
    fn drop(&mut self) {
        let _ = File::unlock(self.0);
    }
}

fn log_path_for(data_directory: &Path, slot: u8) -> PathBuf {
    data_directory.join(log_name_for(slot))
}

fn log_name_for(slot: u8) -> &'static str {
    match slot {
        0 => LOG_FILE,
        1 => ALTERNATE_LOG_FILE,
        _ => unreachable!("validated control slot"),
    }
}

fn initialize_control(lock: &File) -> Result<ControlState, TransactionStoreError> {
    let mut control = lock
        .try_clone()
        .map_err(|source| persistent_io("clone state control file", source))?;
    let length = control
        .metadata()
        .map_err(|source| persistent_io("inspect state control file", source))?
        .len();
    if length < CONTROL_HEADER_BYTES {
        let mut prefix = vec![0_u8; usize::try_from(length).unwrap_or(0)];
        control
            .seek(SeekFrom::Start(0))
            .map_err(|source| persistent_io("seek state control header", source))?;
        control
            .read_exact(&mut prefix)
            .map_err(|source| persistent_io("read partial state control header", source))?;
        if !CONTROL_MAGIC.starts_with(&prefix) {
            return Err(TransactionStoreError::PersistentCorrupt {
                offset: 0,
                reason: "invalid state-control header",
            });
        }
        initialize_control_slots(&mut control)?;
        return read_control(lock);
    }

    control
        .seek(SeekFrom::Start(0))
        .map_err(|source| persistent_io("seek state control header", source))?;
    let mut magic = [0_u8; CONTROL_MAGIC.len()];
    control
        .read_exact(&mut magic)
        .map_err(|source| persistent_io("read state control header", source))?;
    if &magic != CONTROL_MAGIC {
        return Err(TransactionStoreError::PersistentCorrupt {
            offset: 0,
            reason: "invalid state-control header",
        });
    }
    if length != CONTROL_BYTES {
        control
            .set_len(CONTROL_BYTES)
            .map_err(|source| persistent_io("repair state control length", source))?;
        control
            .sync_all()
            .map_err(|source| persistent_io("sync repaired state control length", source))?;
    }
    match read_control(lock) {
        Ok(state) => Ok(state),
        Err(TransactionStoreError::PersistentCorrupt {
            reason: "state-control file has no valid slot",
            ..
        }) if length < CONTROL_BYTES => {
            initialize_control_slots(&mut control)?;
            read_control(lock)
        }
        Err(error) => Err(error),
    }
}

fn read_control(lock: &File) -> Result<ControlState, TransactionStoreError> {
    let mut control = lock
        .try_clone()
        .map_err(|source| persistent_io("clone state control file", source))?;
    let length = control
        .metadata()
        .map_err(|source| persistent_io("inspect state control file", source))?
        .len();
    if length != CONTROL_BYTES {
        return Err(TransactionStoreError::PersistentCorrupt {
            offset: 0,
            reason: "invalid state-control file length",
        });
    }
    control
        .seek(SeekFrom::Start(0))
        .map_err(|source| persistent_io("seek state control header", source))?;
    let mut magic = [0_u8; CONTROL_MAGIC.len()];
    control
        .read_exact(&mut magic)
        .map_err(|source| persistent_io("read state control header", source))?;
    if &magic != CONTROL_MAGIC {
        return Err(TransactionStoreError::PersistentCorrupt {
            offset: 0,
            reason: "invalid state-control header",
        });
    }

    let mut candidates = Vec::with_capacity(CONTROL_SLOT_COUNT);
    for slot_index in 0..CONTROL_SLOT_COUNT {
        let offset = control_slot_offset(slot_index);
        control
            .seek(SeekFrom::Start(offset))
            .map_err(|source| persistent_io("seek state control slot", source))?;
        let mut encoded = [0_u8; CONTROL_RECORD_BYTES];
        control
            .read_exact(&mut encoded)
            .map_err(|source| persistent_io("read state control slot", source))?;
        let (payload, digest) = encoded.split_at(CONTROL_PAYLOAD_BYTES);
        if BlobId::digest(payload).as_bytes().as_slice() != digest {
            continue;
        }
        let candidate = ControlState {
            generation: u64::from_le_bytes(copy_array(&payload[..8])),
            slot: payload[8],
        };
        if usize::from(candidate.slot) != slot_index {
            return Err(TransactionStoreError::PersistentCorrupt {
                offset,
                reason: "invalid state-control slot",
            });
        }
        if usize::try_from(candidate.generation & 1).unwrap_or(usize::MAX) != slot_index {
            return Err(TransactionStoreError::PersistentCorrupt {
                offset,
                reason: "state-control generation is in the wrong slot",
            });
        }
        candidates.push(candidate);
    }
    candidates.sort_unstable_by_key(|candidate| candidate.generation);
    if let [older, newer] = candidates.as_slice()
        && newer.generation != older.generation.saturating_add(1)
    {
        return Err(TransactionStoreError::PersistentCorrupt {
            offset: CONTROL_HEADER_BYTES,
            reason: "state-control generations are not consecutive",
        });
    }
    candidates
        .last()
        .copied()
        .ok_or(TransactionStoreError::PersistentCorrupt {
            offset: CONTROL_HEADER_BYTES,
            reason: "state-control file has no valid slot",
        })
}

fn append_control(
    lock: &File,
    current: ControlState,
    next: ControlState,
) -> Result<(), TransactionStoreError> {
    if next.generation != current.generation.saturating_add(1) || next.slot != (current.slot ^ 1) {
        return Err(TransactionStoreError::PersistentCorrupt {
            offset: CONTROL_HEADER_BYTES,
            reason: "invalid state-control append",
        });
    }
    if read_control(lock)? != current {
        return Err(TransactionStoreError::PersistentCorrupt {
            offset: CONTROL_HEADER_BYTES,
            reason: "state-control changed while exclusively locked",
        });
    }
    let mut control = lock
        .try_clone()
        .map_err(|source| persistent_io("clone state control file", source))?;
    control
        .seek(SeekFrom::Start(control_slot_offset(usize::from(next.slot))))
        .map_err(|source| persistent_io("seek next state control slot", source))?;
    write_control_record(&mut control, next)?;
    control
        .sync_all()
        .map_err(|source| persistent_io("sync state control record", source))
}

fn initialize_control_slots(control: &mut File) -> Result<(), TransactionStoreError> {
    control
        .set_len(0)
        .map_err(|source| persistent_io("reset state control file", source))?;
    control
        .seek(SeekFrom::Start(0))
        .map_err(|source| persistent_io("seek initial state control", source))?;
    control
        .write_all(CONTROL_MAGIC)
        .map_err(|source| persistent_io("write state control header", source))?;
    write_control_record(
        control,
        ControlState {
            generation: 0,
            slot: 0,
        },
    )?;
    control
        .write_all(&[0_u8; CONTROL_RECORD_BYTES])
        .map_err(|source| persistent_io("clear alternate state control slot", source))?;
    control
        .sync_all()
        .map_err(|source| persistent_io("sync initial state control slots", source))
}

fn write_control_record(
    control: &mut File,
    state: ControlState,
) -> Result<(), TransactionStoreError> {
    let mut payload = [0_u8; CONTROL_PAYLOAD_BYTES];
    payload[..8].copy_from_slice(&state.generation.to_le_bytes());
    payload[8] = state.slot;
    control
        .write_all(&payload)
        .and_then(|()| control.write_all(BlobId::digest(&payload).as_bytes()))
        .map_err(|source| persistent_io("write state control record", source))
}

const fn control_slot_offset(slot: usize) -> u64 {
    CONTROL_HEADER_BYTES + (slot * CONTROL_RECORD_BYTES) as u64
}

fn open_log(directory: &Dir, slot: u8) -> Result<File, TransactionStoreError> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    directory::open_real_file(directory, log_name_for(slot), &options)
        .map_err(|source| persistent_io("open state log", source))
}

fn initialize_header(log: &mut File) -> Result<(), TransactionStoreError> {
    let length = log
        .metadata()
        .map_err(|source| persistent_io("inspect state log", source))?
        .len();
    if length == 0 {
        log.write_all(LOG_MAGIC)
            .map_err(|source| persistent_io("write state header", source))?;
        log.sync_all()
            .map_err(|source| persistent_io("sync state header", source))?;
        return Ok(());
    }
    if length < HEADER_BYTES {
        let mut prefix = vec![0_u8; usize::try_from(length).unwrap_or(0)];
        log.seek(SeekFrom::Start(0))
            .map_err(|source| persistent_io("seek state header", source))?;
        log.read_exact(&mut prefix)
            .map_err(|source| persistent_io("read partial state header", source))?;
        if !LOG_MAGIC.starts_with(&prefix) {
            return Err(TransactionStoreError::PersistentCorrupt {
                offset: 0,
                reason: "invalid state-log header",
            });
        }
        log.set_len(0)
            .map_err(|source| persistent_io("repair partial state header", source))?;
        log.seek(SeekFrom::Start(0))
            .map_err(|source| persistent_io("seek repaired state header", source))?;
        log.write_all(LOG_MAGIC)
            .map_err(|source| persistent_io("write repaired state header", source))?;
        log.sync_all()
            .map_err(|source| persistent_io("sync repaired state header", source))?;
        return Ok(());
    }
    let mut magic = [0_u8; LOG_MAGIC.len()];
    log.seek(SeekFrom::Start(0))
        .map_err(|source| persistent_io("seek state header", source))?;
    log.read_exact(&mut magic)
        .map_err(|source| persistent_io("read state header", source))?;
    if &magic != LOG_MAGIC {
        return Err(TransactionStoreError::PersistentCorrupt {
            offset: 0,
            reason: "invalid state-log header",
        });
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn refresh(
    state: &mut PersistentState,
    log: &mut File,
    config: FileStoreConfig,
    control: ControlState,
) -> Result<(), TransactionStoreError> {
    if state.generation != control.generation || state.slot != control.slot {
        state.records.clear();
        state.offset = HEADER_BYTES;
        state.generation = control.generation;
        state.slot = control.slot;
    }
    let log_length = log
        .metadata()
        .map_err(|source| persistent_io("inspect state log", source))?
        .len();
    if log_length > config.max_log_bytes {
        return Err(TransactionStoreError::PersistentLogLimit {
            observed: log_length,
            maximum: config.max_log_bytes,
        });
    }
    if log_length < state.offset || state.offset < HEADER_BYTES {
        return Err(TransactionStoreError::PersistentCorrupt {
            offset: state.offset,
            reason: "state log moved backwards",
        });
    }
    log.seek(SeekFrom::Start(state.offset))
        .map_err(|source| persistent_io("seek state tail", source))?;

    while state.offset < log_length {
        let frame_start = state.offset;
        let mut length_bytes = [0_u8; 4];
        if !read_exact_or_torn(log, &mut length_bytes)? {
            truncate_torn_tail(log, state, frame_start)?;
            break;
        }
        let payload_length =
            usize::try_from(u32::from_le_bytes(length_bytes)).unwrap_or(usize::MAX);
        if !(OLD_MIN_PAYLOAD_BYTES..=MAX_PAYLOAD_BYTES).contains(&payload_length) {
            return Err(TransactionStoreError::PersistentCorrupt {
                offset: frame_start,
                reason: "invalid state-frame length",
            });
        }
        let mut frame = vec![0_u8; payload_length.saturating_add(DIGEST_BYTES)];
        if !read_exact_or_torn(log, &mut frame)? {
            truncate_torn_tail(log, state, frame_start)?;
            break;
        }
        let (payload, encoded_digest) = frame.split_at(payload_length);
        let actual_digest = BlobId::digest(payload);
        if actual_digest.as_bytes().as_slice() != encoded_digest {
            return Err(TransactionStoreError::PersistentCorrupt {
                offset: frame_start,
                reason: "state-frame checksum mismatch",
            });
        }
        let record = decode_record(payload, frame_start)?;
        apply_replayed_record(&mut state.records, record, frame_start, config.max_records)?;
        state.offset = log
            .stream_position()
            .map_err(|source| persistent_io("inspect state tail", source))?;
    }
    Ok(())
}

fn read_exact_or_torn(log: &mut File, output: &mut [u8]) -> Result<bool, TransactionStoreError> {
    match log.read_exact(output) {
        Ok(()) => Ok(true),
        Err(source) if source.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
        Err(source) => Err(persistent_io("read state frame", source)),
    }
}

fn truncate_torn_tail(
    log: &mut File,
    state: &mut PersistentState,
    frame_start: u64,
) -> Result<(), TransactionStoreError> {
    log.set_len(frame_start)
        .map_err(|source| persistent_io("truncate torn state tail", source))?;
    log.sync_all()
        .map_err(|source| persistent_io("sync repaired state tail", source))?;
    log.seek(SeekFrom::Start(frame_start))
        .map_err(|source| persistent_io("seek repaired state tail", source))?;
    state.offset = frame_start;
    Ok(())
}

fn append_record(
    log: &mut File,
    record: &TransactionRecord,
    offset: &mut u64,
    config: FileStoreConfig,
) -> Result<(), TransactionStoreError> {
    let observed = write_record_frame(log, record, *offset, config)?;
    log.sync_all()
        .map_err(|source| persistent_io("sync state frame", source))?;
    *offset = observed;
    Ok(())
}

fn write_record_frame(
    log: &mut File,
    record: &TransactionRecord,
    offset: u64,
    config: FileStoreConfig,
) -> Result<u64, TransactionStoreError> {
    validate_record(record, offset)?;
    let payload = encode_record(record);
    let payload_length =
        u32::try_from(payload.len()).map_err(|_| TransactionStoreError::PersistentCorrupt {
            offset,
            reason: "state record cannot be framed",
        })?;
    let frame_length = 4_u64
        .saturating_add(u64::from(payload_length))
        .saturating_add(DIGEST_BYTES as u64);
    let observed = offset.saturating_add(frame_length);
    if observed > config.max_log_bytes {
        return Err(TransactionStoreError::PersistentLogLimit {
            observed,
            maximum: config.max_log_bytes,
        });
    }
    log.seek(SeekFrom::Start(offset))
        .map_err(|source| persistent_io("seek state append", source))?;
    log.write_all(&payload_length.to_le_bytes())
        .and_then(|()| log.write_all(&payload))
        .and_then(|()| log.write_all(BlobId::digest(&payload).as_bytes()))
        .map_err(|source| persistent_io("append state frame", source))?;
    Ok(observed)
}

fn encode_record(record: &TransactionRecord) -> Vec<u8> {
    let mut payload = Vec::with_capacity(MAX_PAYLOAD_BYTES);
    payload.extend_from_slice(record.id().as_bytes());
    payload.extend_from_slice(record.base_snapshot().as_bytes());
    payload.push(state_tag(record.state()));
    match record.artifact() {
        None => payload.push(0),
        Some(artifact) => {
            payload.push(1);
            payload.extend_from_slice(artifact.as_bytes());
        }
    }
    match record.approval() {
        None => payload.push(0),
        Some(grant) => {
            payload.push(1);
            payload.extend_from_slice(grant.principal().as_bytes());
            payload.extend_from_slice(&grant.issued_at_unix_ms().to_le_bytes());
            payload.extend_from_slice(&grant.expires_at_unix_ms().to_le_bytes());
        }
    }
    payload
}

fn decode_record(payload: &[u8], offset: u64) -> Result<TransactionRecord, TransactionStoreError> {
    if payload.len() == OLD_MIN_PAYLOAD_BYTES || payload.len() == OLD_MAX_PAYLOAD_BYTES {
        return decode_legacy_record(payload, offset);
    }
    let valid_sizes = [
        MIN_PAYLOAD_BYTES,
        MIN_PAYLOAD_BYTES + ARTIFACT_PAYLOAD_BYTES,
        MIN_PAYLOAD_BYTES + APPROVAL_PAYLOAD_BYTES,
        MAX_PAYLOAD_BYTES,
    ];
    if !valid_sizes.contains(&payload.len()) {
        return Err(TransactionStoreError::PersistentCorrupt {
            offset,
            reason: "invalid transaction-record size",
        });
    }
    let id = TransactionId::from_bytes(copy_digest(&payload[..32]));
    let base_snapshot = SnapshotId::from_bytes(copy_digest(&payload[32..64]));
    let state = decode_state(payload[64]).ok_or(TransactionStoreError::PersistentCorrupt {
        offset,
        reason: "unknown transaction state tag",
    })?;
    let mut cursor = 66;
    let artifact = match payload[65] {
        0 => None,
        1 => {
            let end = cursor + ARTIFACT_PAYLOAD_BYTES;
            let bytes =
                payload
                    .get(cursor..end)
                    .ok_or(TransactionStoreError::PersistentCorrupt {
                        offset,
                        reason: "truncated persisted artifact identity",
                    })?;
            cursor = end;
            Some(BlobId::from_bytes(copy_digest(bytes)))
        }
        _ => {
            return Err(TransactionStoreError::PersistentCorrupt {
                offset,
                reason: "invalid persisted artifact encoding",
            });
        }
    };
    let approval_tag = *payload
        .get(cursor)
        .ok_or(TransactionStoreError::PersistentCorrupt {
            offset,
            reason: "missing persisted approval encoding",
        })?;
    cursor += 1;
    let approval = match approval_tag {
        0 if cursor == payload.len() => None,
        1 if cursor + APPROVAL_PAYLOAD_BYTES == payload.len() => {
            let principal = PrincipalId::from_bytes(copy_digest(&payload[cursor..cursor + 32]));
            cursor += 32;
            let issued_at_unix_ms = u64::from_le_bytes(copy_array(&payload[cursor..cursor + 8]));
            cursor += 8;
            let expires_at_unix_ms = u64::from_le_bytes(copy_array(&payload[cursor..cursor + 8]));
            Some(
                ApprovalGrant::new(id, principal, issued_at_unix_ms, expires_at_unix_ms).map_err(
                    |_| TransactionStoreError::PersistentCorrupt {
                        offset,
                        reason: "invalid persisted approval window",
                    },
                )?,
            )
        }
        _ => {
            return Err(TransactionStoreError::PersistentCorrupt {
                offset,
                reason: "invalid persisted approval encoding",
            });
        }
    };
    let record = TransactionRecord {
        id,
        base_snapshot,
        state,
        artifact,
        approval,
    };
    validate_record(&record, offset)?;
    Ok(record)
}

fn decode_legacy_record(
    payload: &[u8],
    offset: u64,
) -> Result<TransactionRecord, TransactionStoreError> {
    let id = TransactionId::from_bytes(copy_digest(&payload[..32]));
    let base_snapshot = SnapshotId::from_bytes(copy_digest(&payload[32..64]));
    let state = decode_state(payload[64]).ok_or(TransactionStoreError::PersistentCorrupt {
        offset,
        reason: "unknown legacy transaction state tag",
    })?;
    let approval = match payload[65] {
        0 if payload.len() == OLD_MIN_PAYLOAD_BYTES => None,
        1 if payload.len() == OLD_MAX_PAYLOAD_BYTES => {
            let principal = PrincipalId::from_bytes(copy_digest(&payload[66..98]));
            let issued_at_unix_ms = u64::from_le_bytes(copy_array(&payload[98..106]));
            let expires_at_unix_ms = u64::from_le_bytes(copy_array(&payload[106..114]));
            Some(
                ApprovalGrant::new(id, principal, issued_at_unix_ms, expires_at_unix_ms).map_err(
                    |_| TransactionStoreError::PersistentCorrupt {
                        offset,
                        reason: "invalid legacy approval window",
                    },
                )?,
            )
        }
        _ => {
            return Err(TransactionStoreError::PersistentCorrupt {
                offset,
                reason: "invalid legacy approval encoding",
            });
        }
    };
    let record = TransactionRecord {
        id,
        base_snapshot,
        state,
        artifact: None,
        approval,
    };
    validate_record(&record, offset)?;
    Ok(record)
}

fn validate_record(record: &TransactionRecord, offset: u64) -> Result<(), TransactionStoreError> {
    if let Some(grant) = record.approval()
        && grant.transaction() != record.id()
    {
        return Err(TransactionStoreError::PersistentCorrupt {
            offset,
            reason: "approval does not bind its transaction",
        });
    }
    let state_forbids_approval = matches!(
        record.state(),
        TransactionState::Created
            | TransactionState::Running
            | TransactionState::VirtualComplete
            | TransactionState::Denied
            | TransactionState::AutoApproved
            | TransactionState::PendingApproval
    );
    if state_forbids_approval && record.approval().is_some() {
        return Err(TransactionStoreError::PersistentCorrupt {
            offset,
            reason: "approval exists before the approved state",
        });
    }
    if matches!(
        record.state(),
        TransactionState::Approved | TransactionState::Expired
    ) && record.approval().is_none()
    {
        return Err(TransactionStoreError::PersistentCorrupt {
            offset,
            reason: "manual approval state lacks its grant",
        });
    }
    Ok(())
}

fn apply_replayed_record(
    records: &mut BTreeMap<TransactionId, TransactionRecord>,
    record: TransactionRecord,
    offset: u64,
    max_records: usize,
) -> Result<(), TransactionStoreError> {
    match records.get(&record.id()) {
        None => {
            if records.len() >= max_records {
                return Err(TransactionStoreError::PersistentRecordLimit {
                    observed: records.len().saturating_add(1),
                    maximum: max_records,
                });
            }
        }
        Some(previous) => {
            if previous.base_snapshot() != record.base_snapshot() {
                return Err(TransactionStoreError::PersistentCorrupt {
                    offset,
                    reason: "transaction base snapshot changed",
                });
            }
            if previous.artifact() != record.artifact() {
                return Err(TransactionStoreError::PersistentCorrupt {
                    offset,
                    reason: "transaction artifact binding changed",
                });
            }
            if !previous.state().can_transition_to(record.state()) {
                return Err(TransactionStoreError::PersistentCorrupt {
                    offset,
                    reason: "invalid persisted transaction transition",
                });
            }
            match (previous.approval(), record.approval()) {
                (None, Some(_))
                    if previous.state() == TransactionState::PendingApproval
                        && record.state() == TransactionState::Approved => {}
                (Some(before), Some(after)) if before == after => {}
                (None, None) => {}
                _ => {
                    return Err(TransactionStoreError::PersistentCorrupt {
                        offset,
                        reason: "persisted approval binding changed",
                    });
                }
            }
        }
    }
    records.insert(record.id(), record);
    Ok(())
}

const fn state_tag(state: TransactionState) -> u8 {
    match state {
        TransactionState::Created => 1,
        TransactionState::Running => 2,
        TransactionState::VirtualComplete => 3,
        TransactionState::Denied => 4,
        TransactionState::AutoApproved => 5,
        TransactionState::PendingApproval => 6,
        TransactionState::Approved => 7,
        TransactionState::Reserved => 8,
        TransactionState::Revalidating => 9,
        TransactionState::Committing => 10,
        TransactionState::Committed => 11,
        TransactionState::Stale => 12,
        TransactionState::Expired => 13,
        TransactionState::RecoveryRequired => 14,
        TransactionState::Failed => 15,
        TransactionState::Rejected => 16,
        _ => 0,
    }
}

const fn decode_state(tag: u8) -> Option<TransactionState> {
    match tag {
        1 => Some(TransactionState::Created),
        2 => Some(TransactionState::Running),
        3 => Some(TransactionState::VirtualComplete),
        4 => Some(TransactionState::Denied),
        5 => Some(TransactionState::AutoApproved),
        6 => Some(TransactionState::PendingApproval),
        7 => Some(TransactionState::Approved),
        8 => Some(TransactionState::Reserved),
        9 => Some(TransactionState::Revalidating),
        10 => Some(TransactionState::Committing),
        11 => Some(TransactionState::Committed),
        12 => Some(TransactionState::Stale),
        13 => Some(TransactionState::Expired),
        14 => Some(TransactionState::RecoveryRequired),
        15 => Some(TransactionState::Failed),
        16 => Some(TransactionState::Rejected),
        _ => None,
    }
}

fn copy_digest(bytes: &[u8]) -> [u8; 32] {
    let mut output = [0_u8; 32];
    output.copy_from_slice(bytes);
    output
}

fn copy_array<const N: usize>(bytes: &[u8]) -> [u8; N] {
    let mut output = [0_u8; N];
    output.copy_from_slice(bytes);
    output
}

fn validate_config(config: FileStoreConfig) -> Result<(), TransactionStoreError> {
    if config.max_log_bytes < HEADER_BYTES {
        return Err(TransactionStoreError::PersistentLogLimit {
            observed: HEADER_BYTES,
            maximum: config.max_log_bytes,
        });
    }
    if config.max_records == 0 {
        return Err(TransactionStoreError::PersistentRecordLimit {
            observed: 1,
            maximum: 0,
        });
    }
    Ok(())
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "map_err owns io::Error; the stable store error intentionally retains only ErrorKind"
)]
fn persistent_io(operation: &'static str, source: io::Error) -> TransactionStoreError {
    TransactionStoreError::PersistentIo {
        operation,
        kind: source.kind(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Barrier};
    use std::thread;

    use vsh_types::{BlobId, PrincipalId, SnapshotId, TransactionId, TransactionState};

    use super::{FileStoreConfig, FileTransactionStore};
    use crate::{ApprovalGrant, TransactionRecord, TransactionStore, TransactionStoreError};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "vsh-file-store-{name}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
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

    fn id(byte: u8) -> TransactionId {
        TransactionId::from_bytes([byte; 32])
    }

    fn snapshot(byte: u8) -> SnapshotId {
        SnapshotId::from_bytes([byte; 32])
    }

    const fn compacting_config() -> FileStoreConfig {
        FileStoreConfig {
            max_log_bytes: 180,
            max_records: 16,
        }
    }

    #[test]
    fn lifecycle_and_approval_survive_reopen() {
        let directory = TestDirectory::new("reopen");
        let store =
            FileTransactionStore::open(directory.path(), FileStoreConfig::default()).unwrap();
        let artifact = BlobId::digest(b"pending artifact");
        let mut record = TransactionRecord::new(id(1), snapshot(2)).with_artifact(artifact);
        record.transition(TransactionState::Running).unwrap();
        record
            .transition(TransactionState::VirtualComplete)
            .unwrap();
        record
            .transition(TransactionState::PendingApproval)
            .unwrap();
        store.create(record).unwrap();
        let grant =
            ApprovalGrant::new(id(1), PrincipalId::digest_label("independent"), 10, 20).unwrap();
        store.approve(id(1), grant).unwrap();

        let reopened =
            FileTransactionStore::open(directory.path(), FileStoreConfig::default()).unwrap();
        let loaded = reopened.get(id(1)).unwrap();
        assert_eq!(loaded.state(), TransactionState::Approved);
        assert_eq!(loaded.artifact(), Some(artifact));
        assert_eq!(loaded.approval(), Some(grant));
        assert_eq!(reopened.reserve(id(1), 11).unwrap().transaction(), id(1));
    }

    #[test]
    fn independent_handles_have_one_cross_process_style_reservation_winner() {
        let directory = TestDirectory::new("reserve");
        let first =
            FileTransactionStore::open(directory.path(), FileStoreConfig::default()).unwrap();
        let second =
            FileTransactionStore::open(directory.path(), FileStoreConfig::default()).unwrap();
        let mut record = TransactionRecord::new(id(3), snapshot(4));
        record.transition(TransactionState::Running).unwrap();
        record
            .transition(TransactionState::VirtualComplete)
            .unwrap();
        record.transition(TransactionState::AutoApproved).unwrap();
        first.create(record).unwrap();

        let barrier = Arc::new(Barrier::new(3));
        let handles = [first, second]
            .into_iter()
            .map(|store| {
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    store.reserve(id(3), 0)
                })
            })
            .collect::<Vec<_>>();
        barrier.wait();
        let results = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(TransactionStoreError::NotReservable { .. })))
                .count(),
            1
        );
    }

    #[test]
    fn torn_tail_is_truncated_to_last_checksummed_state() {
        let directory = TestDirectory::new("torn");
        let store =
            FileTransactionStore::open(directory.path(), FileStoreConfig::default()).unwrap();
        store
            .create(TransactionRecord::new(id(5), snapshot(6)))
            .unwrap();
        let log_path = store.active_log_path().unwrap();
        let valid_length = fs::metadata(&log_path).unwrap().len();
        let mut file = OpenOptions::new().append(true).open(&log_path).unwrap();
        file.write_all(&[114, 0, 0]).unwrap();
        file.sync_all().unwrap();
        drop(store);

        let reopened =
            FileTransactionStore::open(directory.path(), FileStoreConfig::default()).unwrap();
        assert_eq!(
            reopened.get(id(5)).unwrap().state(),
            TransactionState::Created
        );
        assert_eq!(
            fs::metadata(reopened.active_log_path().unwrap())
                .unwrap()
                .len(),
            valid_length
        );
    }

    #[test]
    fn complete_frame_with_bad_checksum_fails_closed_even_at_end_of_log() {
        let directory = TestDirectory::new("checksum");
        let store =
            FileTransactionStore::open(directory.path(), FileStoreConfig::default()).unwrap();
        store
            .create(TransactionRecord::new(id(6), snapshot(7)))
            .unwrap();
        let log_path = store.active_log_path().unwrap();
        drop(store);

        let mut bytes = fs::read(&log_path).unwrap();
        let header_bytes = usize::try_from(super::HEADER_BYTES).unwrap();
        let payload_length =
            u32::from_le_bytes(bytes[header_bytes..header_bytes + 4].try_into().unwrap()) as usize;
        let checksum_start = header_bytes + 4 + payload_length;
        bytes[checksum_start] ^= 0x80;
        fs::write(&log_path, bytes).unwrap();

        assert!(matches!(
            FileTransactionStore::open(directory.path(), FileStoreConfig::default()),
            Err(TransactionStoreError::PersistentCorrupt {
                reason: "state-frame checksum mismatch",
                ..
            })
        ));
    }

    #[test]
    fn partial_matching_header_is_repaired_but_wrong_header_fails_closed() {
        let repairable = TestDirectory::new("partial-header");
        let repairable_log = repairable.path().join(super::LOG_FILE);
        fs::write(&repairable_log, &super::LOG_MAGIC[..4]).unwrap();
        let store =
            FileTransactionStore::open(repairable.path(), FileStoreConfig::default()).unwrap();
        drop(store);
        assert_eq!(
            &fs::read(repairable_log).unwrap()[..super::LOG_MAGIC.len()],
            super::LOG_MAGIC
        );

        let corrupt = TestDirectory::new("wrong-header");
        fs::write(corrupt.path().join(super::LOG_FILE), b"NOTVSH01").unwrap();
        assert!(matches!(
            FileTransactionStore::open(corrupt.path(), FileStoreConfig::default()),
            Err(TransactionStoreError::PersistentCorrupt {
                reason: "invalid state-log header",
                ..
            })
        ));
    }

    #[test]
    fn bounded_compaction_switches_logs_and_stale_handles_replay_the_new_generation() {
        let directory = TestDirectory::new("compact");
        let first = FileTransactionStore::open(directory.path(), compacting_config()).unwrap();
        let second = FileTransactionStore::open(directory.path(), compacting_config()).unwrap();
        first
            .create(TransactionRecord::new(id(7), snapshot(8)))
            .unwrap();
        let initial_log = first.active_log_path().unwrap();

        first
            .compare_and_transition(id(7), TransactionState::Created, TransactionState::Running)
            .unwrap();
        let first_compacted_log = first.active_log_path().unwrap();
        assert_ne!(first_compacted_log, initial_log);
        assert!(
            fs::metadata(&first_compacted_log).unwrap().len() <= compacting_config().max_log_bytes
        );
        assert_eq!(
            second.get(id(7)).unwrap().state(),
            TransactionState::Running
        );

        second
            .compare_and_transition(
                id(7),
                TransactionState::Running,
                TransactionState::VirtualComplete,
            )
            .unwrap();
        assert_eq!(second.active_log_path().unwrap(), initial_log);
        drop(first);
        drop(second);

        let reopened = FileTransactionStore::open(directory.path(), compacting_config()).unwrap();
        assert_eq!(
            reopened.get(id(7)).unwrap().state(),
            TransactionState::VirtualComplete
        );
    }

    #[test]
    fn torn_control_tail_and_inactive_log_never_replace_the_durable_generation() {
        let directory = TestDirectory::new("compact-torn-control");
        let store = FileTransactionStore::open(directory.path(), compacting_config()).unwrap();
        store
            .create(TransactionRecord::new(id(9), snapshot(10)))
            .unwrap();
        let inactive_log = store.active_log_path().unwrap();
        store
            .compare_and_transition(id(9), TransactionState::Created, TransactionState::Running)
            .unwrap();
        let active_log = store.active_log_path().unwrap();
        assert_ne!(active_log, inactive_log);
        drop(store);

        fs::write(&inactive_log, b"uncommitted inactive generation").unwrap();
        let lock_path = directory.path().join(super::LOCK_FILE);
        let valid_control_length = fs::metadata(&lock_path).unwrap().len();
        let mut lock = OpenOptions::new().append(true).open(&lock_path).unwrap();
        lock.write_all(&[0xA5; 7]).unwrap();
        lock.sync_all().unwrap();
        drop(lock);

        let reopened = FileTransactionStore::open(directory.path(), compacting_config()).unwrap();
        assert_eq!(reopened.active_log_path().unwrap(), active_log);
        assert_eq!(
            reopened.get(id(9)).unwrap().state(),
            TransactionState::Running
        );
        assert_eq!(fs::metadata(lock_path).unwrap().len(), valid_control_length);
    }

    #[test]
    fn compaction_that_cannot_fit_all_latest_records_leaves_active_state_unchanged() {
        let directory = TestDirectory::new("compact-overflow");
        let store = FileTransactionStore::open(directory.path(), compacting_config()).unwrap();
        store
            .create(TransactionRecord::new(id(11), snapshot(12)))
            .unwrap();
        let active_log = store.active_log_path().unwrap();

        assert!(matches!(
            store.create(TransactionRecord::new(id(13), snapshot(14))),
            Err(TransactionStoreError::PersistentLogLimit { .. })
        ));
        assert_eq!(store.active_log_path().unwrap(), active_log);
        assert_eq!(
            store.get(id(11)).unwrap().state(),
            TransactionState::Created
        );
        assert!(matches!(
            store.get(id(13)),
            Err(TransactionStoreError::NotFound { .. })
        ));
    }

    #[test]
    fn invalid_store_bounds_fail_before_creating_files() {
        let directory = TestDirectory::new("invalid-bounds");
        let too_small = FileStoreConfig {
            max_log_bytes: 1,
            max_records: 1,
        };
        assert!(matches!(
            FileTransactionStore::open(directory.path(), too_small),
            Err(TransactionStoreError::PersistentLogLimit { .. })
        ));
        let zero_records = FileStoreConfig {
            max_log_bytes: 1024,
            max_records: 0,
        };
        assert!(matches!(
            FileTransactionStore::open(directory.path(), zero_records),
            Err(TransactionStoreError::PersistentRecordLimit { .. })
        ));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn internal_state_file_symlink_cannot_redirect_open() {
        use std::os::unix::fs::symlink;

        let directory = TestDirectory::new("state-symlink");
        let outside = TestDirectory::new("state-symlink-outside");
        let target = outside.path().join("target");
        fs::write(&target, b"outside must remain unchanged").unwrap();
        symlink(&target, directory.path().join(super::LOCK_FILE)).unwrap();

        assert!(matches!(
            FileTransactionStore::open(directory.path(), FileStoreConfig::default()),
            Err(TransactionStoreError::PersistentIo { .. })
        ));
        assert_eq!(fs::read(&target).unwrap(), b"outside must remain unchanged");
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
    }
}
