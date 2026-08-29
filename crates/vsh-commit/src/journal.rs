use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::io::{self, Read, Write};

use cap_std::fs::{Dir, File};
use vsh_types::{FileStamp, NodeKind, PlatformFileId, TransactionId};

use crate::host::{create_new_file, open_real_file, sync_dir};

const JOURNAL_MAGIC: &[u8; 8] = b"VSHLOG01";
const MARKER_MAGIC: &[u8; 8] = b"VSHDONE1";
const MAX_RECORD_PAYLOAD: usize = 64;

pub(crate) const PLAN_FILE: &str = "plan";
pub(crate) const JOURNAL_FILE: &str = "journal";
pub(crate) const COMMIT_MARKER: &str = "commit-complete";
pub(crate) const STAGE_DIRECTORY: &str = "stage";
pub(crate) const QUARANTINE_DIRECTORY: &str = "quarantine";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Witness {
    pub(crate) kind: NodeKind,
    pub(crate) file_id: PlatformFileId,
}

impl From<FileStamp> for Witness {
    fn from(stamp: FileStamp) -> Self {
        Self {
            kind: stamp.kind,
            file_id: stamp.file_id,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Journal {
    file: File,
}

impl Journal {
    pub(crate) fn create(transaction_dir: &Dir) -> Result<Self, io::Error> {
        let mut file = create_new_file(transaction_dir, JOURNAL_FILE)?;
        file.write_all(JOURNAL_MAGIC)?;
        file.sync_all()?;
        sync_dir(transaction_dir)?;
        Ok(Self { file })
    }

    pub(crate) fn intent(
        &mut self,
        index: u32,
        source_witness: Option<Witness>,
        parent_witness: Witness,
    ) -> Result<(), io::Error> {
        let mut payload = Vec::with_capacity(40);
        payload.push(1);
        payload.extend_from_slice(&index.to_le_bytes());
        payload.push(u8::from(source_witness.is_some()) | 2);
        if let Some(witness) = source_witness {
            encode_witness(witness, &mut payload);
        }
        encode_witness(parent_witness, &mut payload);
        self.append_record(&payload)
    }

    pub(crate) fn done(&mut self, index: u32, witness: Witness) -> Result<(), io::Error> {
        let mut payload = Vec::with_capacity(22);
        payload.push(2);
        payload.extend_from_slice(&index.to_le_bytes());
        payload.push(witness.kind.canonical_tag());
        payload.extend_from_slice(&witness.file_id.high.to_le_bytes());
        payload.extend_from_slice(&witness.file_id.low.to_le_bytes());
        self.append_record(&payload)
    }

    fn append_record(&mut self, payload: &[u8]) -> Result<(), io::Error> {
        let len = u32::try_from(payload.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "journal record too large"))?;
        self.file.write_all(&len.to_le_bytes())?;
        self.file.write_all(payload)?;
        self.file.write_all(&record_digest(payload))?;
        self.file.sync_all()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct JournalState {
    pub(crate) completed: BTreeMap<u32, Witness>,
    intents: BTreeMap<u32, IntentWitnesses>,
    pending: Option<u32>,
    pub(crate) torn_tail: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct IntentWitnesses {
    source: Option<Witness>,
    parent: Option<Witness>,
}

impl JournalState {
    pub(crate) fn has_intent(&self, index: u32) -> bool {
        self.intents.contains_key(&index)
    }

    pub(crate) fn witness(&self, index: u32) -> Option<Witness> {
        self.completed.get(&index).copied()
    }

    pub(crate) fn intent_witness(&self, index: u32) -> Option<Witness> {
        self.intents
            .get(&index)
            .and_then(|witnesses| witnesses.source)
    }

    pub(crate) fn parent_witness(&self, index: u32) -> Option<Witness> {
        self.intents
            .get(&index)
            .and_then(|witnesses| witnesses.parent)
    }
}

pub(crate) fn read_journal(
    transaction_dir: &Dir,
    maximum_bytes: usize,
) -> Result<JournalState, JournalError> {
    let mut file = open_real_file(transaction_dir, JOURNAL_FILE).map_err(JournalError::Io)?;
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(
            u64::try_from(maximum_bytes)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)
        .map_err(JournalError::Io)?;
    if bytes.len() > maximum_bytes {
        return Err(JournalError::RecordLength);
    }
    if bytes.len() < JOURNAL_MAGIC.len() || &bytes[..8] != JOURNAL_MAGIC {
        return Err(JournalError::Magic);
    }
    let mut offset = 8_usize;
    let mut state = JournalState::default();
    let mut next_index = 0_u32;
    while offset < bytes.len() {
        let Some(length_bytes) = bytes.get(offset..offset.saturating_add(4)) else {
            state.torn_tail = true;
            break;
        };
        let length = u32::from_le_bytes(
            length_bytes
                .try_into()
                .expect("four-byte journal length slice"),
        ) as usize;
        if length > MAX_RECORD_PAYLOAD {
            return Err(JournalError::RecordLength);
        }
        let payload_start = offset + 4;
        let payload_end = payload_start
            .checked_add(length)
            .ok_or(JournalError::RecordLength)?;
        let checksum_end = payload_end
            .checked_add(32)
            .ok_or(JournalError::RecordLength)?;
        let Some(payload) = bytes.get(payload_start..payload_end) else {
            state.torn_tail = true;
            break;
        };
        let Some(checksum) = bytes.get(payload_end..checksum_end) else {
            state.torn_tail = true;
            break;
        };
        if record_digest(payload).as_slice() != checksum {
            return Err(JournalError::Checksum);
        }
        match payload.first().copied() {
            Some(1) if matches!(payload.len(), 5 | 22 | 23 | 40) => {
                let index = u32::from_le_bytes(
                    payload[1..5]
                        .try_into()
                        .expect("four-byte intent index slice"),
                );
                if state.pending.is_some() || index != next_index {
                    return Err(JournalError::Sequence);
                }
                let witnesses = match payload.len() {
                    5 => IntentWitnesses::default(),
                    22 => IntentWitnesses {
                        source: Some(decode_witness_at(payload, 5)?),
                        parent: None,
                    },
                    23 | 40 => decode_intent_witnesses(payload)?,
                    _ => unreachable!("guarded intent payload length"),
                };
                state.intents.insert(index, witnesses);
                state.pending = Some(index);
            }
            Some(2) if payload.len() == 22 => {
                let index = u32::from_le_bytes(
                    payload[1..5]
                        .try_into()
                        .expect("four-byte completion index slice"),
                );
                if state.pending != Some(index) || index != next_index {
                    return Err(JournalError::Sequence);
                }
                state.completed.insert(index, decode_witness(payload)?);
                state.pending = None;
                next_index = next_index.checked_add(1).ok_or(JournalError::Sequence)?;
            }
            _ => return Err(JournalError::Tag),
        }
        offset = checksum_end;
    }
    Ok(state)
}

fn decode_witness(payload: &[u8]) -> Result<Witness, JournalError> {
    decode_witness_at(payload, 5)
}

fn encode_witness(witness: Witness, output: &mut Vec<u8>) {
    output.push(witness.kind.canonical_tag());
    output.extend_from_slice(&witness.file_id.high.to_le_bytes());
    output.extend_from_slice(&witness.file_id.low.to_le_bytes());
}

fn decode_intent_witnesses(payload: &[u8]) -> Result<IntentWitnesses, JournalError> {
    let flags = payload[5];
    if flags & !3 != 0 || flags & 2 == 0 {
        return Err(JournalError::Tag);
    }
    let mut offset = 6;
    let source = if flags & 1 == 1 {
        let witness = decode_witness_at(payload, offset)?;
        offset += 17;
        Some(witness)
    } else {
        None
    };
    let parent = Some(decode_witness_at(payload, offset)?);
    offset += 17;
    if offset != payload.len() {
        return Err(JournalError::RecordLength);
    }
    Ok(IntentWitnesses { source, parent })
}

fn decode_witness_at(payload: &[u8], offset: usize) -> Result<Witness, JournalError> {
    let bytes = payload
        .get(offset..offset.saturating_add(17))
        .ok_or(JournalError::RecordLength)?;
    let kind = match bytes[0] {
        1 => NodeKind::File,
        2 => NodeKind::Directory,
        3 => NodeKind::Symlink,
        _ => return Err(JournalError::Tag),
    };
    let high = u64::from_le_bytes(
        bytes[1..9]
            .try_into()
            .expect("eight-byte witness high slice"),
    );
    let low = u64::from_le_bytes(
        bytes[9..17]
            .try_into()
            .expect("eight-byte witness low slice"),
    );
    Ok(Witness {
        kind,
        file_id: PlatformFileId { high, low },
    })
}

pub(crate) fn write_commit_marker(
    transaction_dir: &Dir,
    transaction: TransactionId,
) -> Result<(), io::Error> {
    let mut payload = Vec::with_capacity(72);
    payload.extend_from_slice(MARKER_MAGIC);
    payload.extend_from_slice(transaction.as_bytes());
    let digest = marker_digest(&payload);
    payload.extend_from_slice(&digest);
    let mut marker = create_new_file(transaction_dir, COMMIT_MARKER)?;
    marker.write_all(&payload)?;
    marker.sync_all()?;
    sync_dir(transaction_dir)
}

pub(crate) fn has_valid_commit_marker(
    transaction_dir: &Dir,
    transaction: TransactionId,
) -> Result<bool, JournalError> {
    let mut marker = match open_real_file(transaction_dir, COMMIT_MARKER) {
        Ok(marker) => marker,
        Err(source) if source.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(source) => return Err(JournalError::Io(source)),
    };
    let mut bytes = Vec::new();
    Read::by_ref(&mut marker)
        .take(73)
        .read_to_end(&mut bytes)
        .map_err(JournalError::Io)?;
    if bytes.len() != 72 || &bytes[..8] != MARKER_MAGIC {
        return Err(JournalError::Marker);
    }
    if &bytes[8..40] != transaction.as_bytes() {
        return Err(JournalError::Marker);
    }
    if marker_digest(&bytes[..40]).as_slice() != &bytes[40..72] {
        return Err(JournalError::Marker);
    }
    Ok(true)
}

fn record_digest(payload: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"vsh\0commit-journal-record-v1\0");
    hasher.update(&(payload.len() as u64).to_le_bytes());
    hasher.update(payload);
    *hasher.finalize().as_bytes()
}

fn marker_digest(payload: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"vsh\0commit-marker-v1\0");
    hasher.update(payload);
    *hasher.finalize().as_bytes()
}

/// Durable operation-journal or commit-marker validation failure.
#[derive(Debug)]
pub enum JournalError {
    /// Journal I/O failed.
    Io(io::Error),
    /// Journal header is unknown.
    Magic,
    /// A framed record length is invalid or exceeds its bound.
    RecordLength,
    /// A complete record failed its BLAKE3 checksum.
    Checksum,
    /// Intent/completion records are not strictly sequential.
    Sequence,
    /// A record contains an unknown semantic tag.
    Tag,
    /// The durable commit-complete marker is invalid.
    Marker,
}

impl fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Io(_) => "commit journal I/O failed",
            Self::Magic => "commit journal has an unknown format",
            Self::RecordLength => "commit journal record length is invalid",
            Self::Checksum => "commit journal checksum mismatch",
            Self::Sequence => "commit journal operation sequence is invalid",
            Self::Tag => "commit journal contains an unknown record tag",
            Self::Marker => "commit-complete marker is invalid",
        })
    }
}

impl Error for JournalError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(source) => Some(source),
            Self::Magic
            | Self::RecordLength
            | Self::Checksum
            | Self::Sequence
            | Self::Tag
            | Self::Marker => None,
        }
    }
}
