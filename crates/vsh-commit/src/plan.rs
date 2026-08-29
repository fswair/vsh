use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use vsh_policy::{read_set_digest, write_set_digest};
use vsh_types::{
    BlobId, ContentVersion, FileStamp, NodeKind, NodeState, PlatformFileId, SnapshotId,
    TransactionBinding, TransactionId, VPath,
};
use vsh_vfs::{CanonicalDiff, ReadObservation, WritePrecondition};

const PLAN_MAGIC: &[u8; 8] = b"VSHCMT01";

/// Borrowed exact transaction artifact accepted by the trusted committer.
pub struct CommitPlan<'a> {
    binding: TransactionBinding,
    diff: &'a CanonicalDiff,
    read_set: &'a BTreeMap<VPath, ReadObservation>,
    write_set: &'a BTreeMap<VPath, WritePrecondition>,
}

impl<'a> CommitPlan<'a> {
    /// Validate that diff and dependency digests match the transaction binding.
    ///
    /// # Errors
    ///
    /// Returns an error for digest mismatch, reserved paths, missing write
    /// preconditions, or unmaterialized final content.
    ///
    /// # Panics
    ///
    /// Panics only if the compile-time trusted runtime-directory name stops satisfying
    /// [`VPath`] rules.
    pub fn new(
        binding: &TransactionBinding,
        diff: &'a CanonicalDiff,
        read_set: &'a BTreeMap<VPath, ReadObservation>,
        write_set: &'a BTreeMap<VPath, WritePrecondition>,
    ) -> Result<Self, CommitPlanError> {
        if binding.diff != diff.digest() {
            return Err(CommitPlanError::DiffDigestMismatch);
        }
        if binding.read_set != read_set_digest(read_set) {
            return Err(CommitPlanError::ReadSetDigestMismatch);
        }
        if binding.write_set != write_set_digest(write_set) {
            return Err(CommitPlanError::WriteSetDigestMismatch);
        }
        let reserved = VPath::parse(crate::host::RUNTIME_DIRECTORY)
            .expect("built-in runtime directory is a valid VPath");
        for entry in diff.entries() {
            if entry.path.is_root() {
                return Err(CommitPlanError::RootMutation);
            }
            if entry.path.is_within(&reserved) {
                return Err(CommitPlanError::ReservedPath {
                    path: entry.path.clone(),
                });
            }
            let parent = entry
                .path
                .parent()
                .expect("non-root diff paths have a parent");
            if read_set
                .get(&parent)
                .is_none_or(|observation| observation.metadata.is_none())
            {
                return Err(CommitPlanError::MissingParentDependency {
                    path: entry.path.clone(),
                    parent,
                });
            }
            let Some(precondition) = write_set.get(&entry.path) else {
                return Err(CommitPlanError::MissingWritePrecondition {
                    path: entry.path.clone(),
                });
            };
            if precondition.expected != entry.before {
                // A lazily materialized before-state legitimately differs only by replacing
                // its stamp with the exact blob. The precondition deliberately retains the
                // stronger original host identity.
                let materialized_stamp = matches!(
                    (precondition.expected, entry.before),
                    (
                        Some(expected),
                        Some(before)
                    ) if matches!(expected.content(), Some(ContentVersion::Stamp(_)))
                        && matches!(before.content(), Some(ContentVersion::Blob(_)))
                        && expected.kind() == before.kind()
                        && expected.size() == before.size()
                        && expected.mode() == before.mode()
                );
                if !materialized_stamp {
                    return Err(CommitPlanError::BeforeStateMismatch {
                        path: entry.path.clone(),
                    });
                }
            }
            if let Some(after) = entry.after
                && after.kind() != NodeKind::Directory
                && !matches!(after.content(), Some(ContentVersion::Blob(_)))
            {
                return Err(CommitPlanError::UnmaterializedAfterState {
                    path: entry.path.clone(),
                });
            }
        }
        Ok(Self {
            binding: *binding,
            diff,
            read_set,
            write_set,
        })
    }

    #[must_use]
    /// Return the transaction ID derived from the exact binding.
    pub fn transaction(&self) -> TransactionId {
        self.binding.transaction_id()
    }

    #[must_use]
    /// Return the immutable base snapshot identity.
    pub const fn base_snapshot(&self) -> SnapshotId {
        self.binding.base_snapshot
    }

    #[must_use]
    /// Return the complete identity binding.
    pub const fn binding(&self) -> TransactionBinding {
        self.binding
    }

    #[must_use]
    /// Return the canonical final diff.
    pub const fn diff(&self) -> &CanonicalDiff {
        self.diff
    }

    #[must_use]
    /// Return dependencies observed by virtual execution.
    pub const fn read_set(&self) -> &BTreeMap<VPath, ReadObservation> {
        self.read_set
    }

    #[must_use]
    /// Return base preconditions for all virtual writes.
    pub const fn write_set(&self) -> &BTreeMap<VPath, WritePrecondition> {
        self.write_set
    }
}

/// Invalid or unbounded immutable commit artifact.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommitPlanError {
    /// Canonical diff digest does not match the transaction binding.
    DiffDigestMismatch,
    /// `ReadSet` digest does not match the transaction binding.
    ReadSetDigestMismatch,
    /// `WriteSet` digest does not match the transaction binding.
    WriteSetDigestMismatch,
    /// The diff attempts to replace the workspace root.
    RootMutation,
    /// The diff targets trusted runtime state.
    ReservedPath {
        /// Rejected path.
        path: VPath,
    },
    /// A changed path lacks a base write precondition.
    MissingWritePrecondition {
        /// Affected path.
        path: VPath,
    },
    /// A changed path lacks the parent-directory identity needed for safe `*at` access.
    MissingParentDependency {
        /// Changed path.
        path: VPath,
        /// Parent directory that was not recorded.
        parent: VPath,
    },
    /// Diff and precondition disagree about base state.
    BeforeStateMismatch {
        /// Affected path.
        path: VPath,
    },
    /// Final file or link content is not immutable and blob-backed.
    UnmaterializedAfterState {
        /// Affected path.
        path: VPath,
    },
    /// Lowered operation count exceeds the configured maximum.
    TooManyOperations {
        /// Observed count.
        observed: usize,
        /// Configured maximum.
        maximum: usize,
    },
    /// A normalized path exceeds the configured byte bound.
    PathTooLong {
        /// Rejected path.
        path: VPath,
        /// Configured maximum.
        maximum: usize,
    },
    /// An operation count cannot fit the stable journal codec.
    OperationCountOverflow,
}

impl fmt::Display for CommitPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DiffDigestMismatch => {
                formatter.write_str("commit diff digest does not match its transaction binding")
            }
            Self::ReadSetDigestMismatch => {
                formatter.write_str("commit read-set digest does not match its transaction binding")
            }
            Self::WriteSetDigestMismatch => formatter
                .write_str("commit write-set digest does not match its transaction binding"),
            Self::RootMutation => formatter.write_str("the workspace root cannot be mutated"),
            Self::ReservedPath { path } => {
                write!(formatter, "commit targets reserved VSH path {path}")
            }
            Self::MissingWritePrecondition { path } => {
                write!(formatter, "diff path {path} has no write precondition")
            }
            Self::MissingParentDependency { path, parent } => write!(
                formatter,
                "diff path {path} has no metadata dependency for parent {parent}"
            ),
            Self::BeforeStateMismatch { path } => write!(
                formatter,
                "diff before-state and write precondition disagree at {path}"
            ),
            Self::UnmaterializedAfterState { path } => {
                write!(formatter, "commit after-state at {path} is not blob-backed")
            }
            Self::TooManyOperations { observed, maximum } => write!(
                formatter,
                "commit has {observed} operations; maximum is {maximum}"
            ),
            Self::PathTooLong { path, maximum } => write!(
                formatter,
                "commit path {path} exceeds the {maximum}-byte limit"
            ),
            Self::OperationCountOverflow => {
                formatter.write_str("commit operation count cannot be encoded")
            }
        }
    }
}

impl Error for CommitPlanError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Operation {
    Quarantine {
        path: VPath,
        expected: NodeState,
        slot: u32,
    },
    CreateDirectory {
        path: VPath,
        after: NodeState,
    },
    InstallFile {
        path: VPath,
        after: NodeState,
        slot: u32,
    },
    InstallSymlink {
        path: VPath,
        after: NodeState,
        slot: u32,
    },
    SetDirectoryMode {
        path: VPath,
        expected: NodeState,
        after_mode: u32,
    },
}

impl Operation {
    pub(crate) fn path(&self) -> &VPath {
        match self {
            Self::Quarantine { path, .. }
            | Self::CreateDirectory { path, .. }
            | Self::InstallFile { path, .. }
            | Self::InstallSymlink { path, .. }
            | Self::SetDirectoryMode { path, .. } => path,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PreparedPlan {
    pub(crate) transaction: TransactionId,
    pub(crate) base_snapshot: SnapshotId,
    pub(crate) operations: Vec<Operation>,
    pub(crate) final_states: Vec<(VPath, Option<NodeState>)>,
}

impl PreparedPlan {
    #[allow(clippy::too_many_lines)]
    pub(crate) fn prepare(
        plan: &CommitPlan<'_>,
        max_operations: usize,
        max_path_bytes: usize,
    ) -> Result<Self, CommitPlanError> {
        let mut destructive = Vec::new();
        for entry in plan.diff.entries() {
            let Some(before) = entry.before else {
                continue;
            };
            let destructive_change = match entry.after {
                None => true,
                Some(after) if after.kind() != before.kind() => true,
                Some(after) => matches!(after.kind(), NodeKind::File | NodeKind::Symlink),
            };
            if destructive_change {
                destructive.push(entry.path.clone());
            }
        }
        destructive.sort_unstable_by(|left, right| {
            component_depth(left)
                .cmp(&component_depth(right))
                .then_with(|| left.cmp(right))
        });
        let mut destructive_roots = Vec::new();
        for path in destructive {
            if destructive_roots
                .iter()
                .any(|ancestor: &VPath| path.is_within(ancestor))
            {
                continue;
            }
            destructive_roots.push(path);
        }

        let mut operations = Vec::new();
        let mut next_slot = 0_u32;
        for path in &destructive_roots {
            let expected = plan.write_set[path]
                .expected
                .expect("destructive diff entries have an existing precondition");
            operations.push(Operation::Quarantine {
                path: path.clone(),
                expected,
                slot: next_slot,
            });
            next_slot = next_slot
                .checked_add(1)
                .ok_or(CommitPlanError::OperationCountOverflow)?;
        }

        let mut directories = plan
            .diff
            .entries()
            .iter()
            .filter_map(|entry| {
                let after = entry.after?;
                let needs_create = after.kind() == NodeKind::Directory
                    && entry
                        .before
                        .is_none_or(|before| before.kind() != NodeKind::Directory);
                needs_create.then(|| (entry.path.clone(), NodeState::directory(after.mode())))
            })
            .collect::<Vec<_>>();
        directories.sort_unstable_by(|left, right| {
            component_depth(&left.0)
                .cmp(&component_depth(&right.0))
                .then_with(|| left.0.cmp(&right.0))
        });
        for (path, after) in directories {
            operations.push(Operation::CreateDirectory { path, after });
        }

        for entry in plan.diff.entries() {
            let Some(after) = entry.after else {
                continue;
            };
            match after.kind() {
                NodeKind::File => {
                    operations.push(Operation::InstallFile {
                        path: entry.path.clone(),
                        after,
                        slot: next_slot,
                    });
                    next_slot = next_slot
                        .checked_add(1)
                        .ok_or(CommitPlanError::OperationCountOverflow)?;
                }
                NodeKind::Symlink => {
                    operations.push(Operation::InstallSymlink {
                        path: entry.path.clone(),
                        after,
                        slot: next_slot,
                    });
                    next_slot = next_slot
                        .checked_add(1)
                        .ok_or(CommitPlanError::OperationCountOverflow)?;
                }
                NodeKind::Directory => {
                    if let Some(before) = entry.before
                        && before.kind() == NodeKind::Directory
                        && before.mode() != after.mode()
                    {
                        operations.push(Operation::SetDirectoryMode {
                            path: entry.path.clone(),
                            expected: plan.write_set[&entry.path]
                                .expected
                                .expect("directory metadata change has a precondition"),
                            after_mode: after.mode(),
                        });
                    }
                }
            }
        }

        if operations.len() > max_operations {
            return Err(CommitPlanError::TooManyOperations {
                observed: operations.len(),
                maximum: max_operations,
            });
        }
        for path in operations.iter().map(Operation::path) {
            if path.as_str().len() > max_path_bytes {
                return Err(CommitPlanError::PathTooLong {
                    path: path.clone(),
                    maximum: max_path_bytes,
                });
            }
        }
        let final_states = plan
            .diff
            .entries()
            .iter()
            .map(|entry| {
                let after = entry.after.map(|state| {
                    if state.kind() == NodeKind::Directory {
                        NodeState::directory(state.mode())
                    } else {
                        state
                    }
                });
                (entry.path.clone(), after)
            })
            .collect();
        Ok(Self {
            transaction: plan.transaction(),
            base_snapshot: plan.base_snapshot(),
            operations,
            final_states,
        })
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>, CommitPlanError> {
        let operation_count = u32::try_from(self.operations.len())
            .map_err(|_| CommitPlanError::OperationCountOverflow)?;
        let final_count = u32::try_from(self.final_states.len())
            .map_err(|_| CommitPlanError::OperationCountOverflow)?;
        let mut output = Vec::new();
        output.extend_from_slice(PLAN_MAGIC);
        output.extend_from_slice(self.transaction.as_bytes());
        output.extend_from_slice(self.base_snapshot.as_bytes());
        output.extend_from_slice(&operation_count.to_le_bytes());
        for operation in &self.operations {
            encode_operation(operation, &mut output)?;
        }
        output.extend_from_slice(&final_count.to_le_bytes());
        for (path, state) in &self.final_states {
            encode_path(path, &mut output)?;
            encode_optional_state(*state, &mut output);
        }
        let digest = plan_digest(&output);
        output.extend_from_slice(&digest);
        Ok(output)
    }

    pub(crate) fn decode(
        bytes: &[u8],
        max_operations: usize,
        max_path_bytes: usize,
    ) -> Result<Self, PlanDecodeError> {
        if bytes.len() < PLAN_MAGIC.len() + 32 {
            return Err(PlanDecodeError::Truncated);
        }
        let (payload, checksum) = bytes.split_at(bytes.len() - 32);
        if plan_digest(payload).as_slice() != checksum {
            return Err(PlanDecodeError::Checksum);
        }
        let mut reader = Reader::new(payload);
        if reader.take(8)? != PLAN_MAGIC {
            return Err(PlanDecodeError::Magic);
        }
        let transaction = TransactionId::from_bytes(reader.array()?);
        let base_snapshot = SnapshotId::from_bytes(reader.array()?);
        let operation_count = reader.u32()? as usize;
        if operation_count > max_operations {
            return Err(PlanDecodeError::Limit);
        }
        let mut operations = Vec::with_capacity(operation_count);
        for _ in 0..operation_count {
            operations.push(decode_operation(&mut reader, max_path_bytes)?);
        }
        let final_count = reader.u32()? as usize;
        if final_count > max_operations {
            return Err(PlanDecodeError::Limit);
        }
        let mut final_states = Vec::with_capacity(final_count);
        for _ in 0..final_count {
            let path = decode_path(&mut reader, max_path_bytes)?;
            let state = decode_optional_state(&mut reader)?;
            final_states.push((path, state));
        }
        if !reader.is_empty() {
            return Err(PlanDecodeError::TrailingBytes);
        }
        Ok(Self {
            transaction,
            base_snapshot,
            operations,
            final_states,
        })
    }
}

fn component_depth(path: &VPath) -> usize {
    if path.is_root() {
        0
    } else {
        path.as_str().split('/').count()
    }
}

fn slot_name(slot: u32) -> String {
    format!("{slot:08x}")
}

pub(crate) fn stage_name(slot: u32) -> String {
    slot_name(slot)
}

pub(crate) fn stage_link_name(slot: u32) -> String {
    format!("{}.link", slot_name(slot))
}

pub(crate) fn quarantine_name(slot: u32) -> String {
    slot_name(slot)
}

fn encode_operation(operation: &Operation, output: &mut Vec<u8>) -> Result<(), CommitPlanError> {
    match operation {
        Operation::Quarantine {
            path,
            expected,
            slot,
        } => {
            output.push(1);
            encode_path(path, output)?;
            encode_state(*expected, output);
            output.extend_from_slice(&slot.to_le_bytes());
        }
        Operation::CreateDirectory { path, after } => {
            output.push(2);
            encode_path(path, output)?;
            encode_state(*after, output);
        }
        Operation::InstallFile { path, after, slot } => {
            output.push(3);
            encode_path(path, output)?;
            encode_state(*after, output);
            output.extend_from_slice(&slot.to_le_bytes());
        }
        Operation::InstallSymlink { path, after, slot } => {
            output.push(4);
            encode_path(path, output)?;
            encode_state(*after, output);
            output.extend_from_slice(&slot.to_le_bytes());
        }
        Operation::SetDirectoryMode {
            path,
            expected,
            after_mode,
        } => {
            output.push(5);
            encode_path(path, output)?;
            encode_state(*expected, output);
            output.extend_from_slice(&after_mode.to_le_bytes());
        }
    }
    Ok(())
}

fn decode_operation(
    reader: &mut Reader<'_>,
    max_path: usize,
) -> Result<Operation, PlanDecodeError> {
    let tag = reader.u8()?;
    let path = decode_path(reader, max_path)?;
    match tag {
        1 => Ok(Operation::Quarantine {
            path,
            expected: decode_state(reader)?,
            slot: reader.u32()?,
        }),
        2 => Ok(Operation::CreateDirectory {
            path,
            after: decode_state(reader)?,
        }),
        3 => Ok(Operation::InstallFile {
            path,
            after: decode_state(reader)?,
            slot: reader.u32()?,
        }),
        4 => Ok(Operation::InstallSymlink {
            path,
            after: decode_state(reader)?,
            slot: reader.u32()?,
        }),
        5 => Ok(Operation::SetDirectoryMode {
            path,
            expected: decode_state(reader)?,
            after_mode: reader.u32()?,
        }),
        _ => Err(PlanDecodeError::Tag),
    }
}

fn encode_path(path: &VPath, output: &mut Vec<u8>) -> Result<(), CommitPlanError> {
    let bytes = path.as_str().as_bytes();
    let len = u32::try_from(bytes.len()).map_err(|_| CommitPlanError::PathTooLong {
        path: path.clone(),
        maximum: u32::MAX as usize,
    })?;
    output.extend_from_slice(&len.to_le_bytes());
    output.extend_from_slice(bytes);
    Ok(())
}

fn decode_path(reader: &mut Reader<'_>, max_path: usize) -> Result<VPath, PlanDecodeError> {
    let len = reader.u32()? as usize;
    if len > max_path {
        return Err(PlanDecodeError::Limit);
    }
    let source = std::str::from_utf8(reader.take(len)?).map_err(|_| PlanDecodeError::Utf8)?;
    VPath::parse(source).map_err(|_| PlanDecodeError::Path)
}

fn encode_optional_state(state: Option<NodeState>, output: &mut Vec<u8>) {
    match state {
        None => output.push(0),
        Some(state) => {
            output.push(1);
            encode_state(state, output);
        }
    }
}

fn decode_optional_state(reader: &mut Reader<'_>) -> Result<Option<NodeState>, PlanDecodeError> {
    match reader.u8()? {
        0 => Ok(None),
        1 => decode_state(reader).map(Some),
        _ => Err(PlanDecodeError::Tag),
    }
}

fn encode_state(state: NodeState, output: &mut Vec<u8>) {
    output.push(state.kind().canonical_tag());
    output.extend_from_slice(&state.size().to_le_bytes());
    output.extend_from_slice(&state.mode().to_le_bytes());
    match state.content() {
        None => output.push(0),
        Some(ContentVersion::Blob(blob)) => {
            output.push(1);
            output.extend_from_slice(blob.as_bytes());
        }
        Some(ContentVersion::Stamp(stamp)) => {
            output.push(2);
            encode_stamp(stamp, output);
        }
        Some(_) => unreachable!("all vsh-types content versions are explicitly encoded"),
    }
}

fn decode_state(reader: &mut Reader<'_>) -> Result<NodeState, PlanDecodeError> {
    let kind = decode_kind(reader.u8()?)?;
    let size = reader.u64()?;
    let mode = reader.u32()?;
    match reader.u8()? {
        0 if kind == NodeKind::Directory && size == 0 => Ok(NodeState::directory(mode)),
        1 => {
            let blob = BlobId::from_bytes(reader.array()?);
            match kind {
                NodeKind::File => Ok(NodeState::file(blob, size, mode)),
                NodeKind::Symlink => Ok(NodeState::symlink(blob, size, mode)),
                NodeKind::Directory => Err(PlanDecodeError::State),
            }
        }
        2 => {
            let stamp = decode_stamp(reader)?;
            if stamp.kind != kind || stamp.size != size || stamp.mode != mode {
                return Err(PlanDecodeError::State);
            }
            Ok(NodeState::from_stamp(stamp))
        }
        _ => Err(PlanDecodeError::State),
    }
}

fn encode_stamp(stamp: FileStamp, output: &mut Vec<u8>) {
    output.push(stamp.kind.canonical_tag());
    output.extend_from_slice(&stamp.size.to_le_bytes());
    output.extend_from_slice(&stamp.mode.to_le_bytes());
    output.extend_from_slice(&stamp.mtime_ns.to_le_bytes());
    match stamp.ctime_ns {
        None => output.push(0),
        Some(ctime) => {
            output.push(1);
            output.extend_from_slice(&ctime.to_le_bytes());
        }
    }
    output.extend_from_slice(&stamp.file_id.high.to_le_bytes());
    output.extend_from_slice(&stamp.file_id.low.to_le_bytes());
}

fn decode_stamp(reader: &mut Reader<'_>) -> Result<FileStamp, PlanDecodeError> {
    let kind = decode_kind(reader.u8()?)?;
    let size = reader.u64()?;
    let mode = reader.u32()?;
    let mtime_ns = reader.i128()?;
    let ctime_ns = match reader.u8()? {
        0 => None,
        1 => Some(reader.i128()?),
        _ => return Err(PlanDecodeError::Tag),
    };
    let high = reader.u64()?;
    let low = reader.u64()?;
    Ok(FileStamp {
        kind,
        size,
        mode,
        mtime_ns,
        ctime_ns,
        file_id: PlatformFileId { high, low },
    })
}

fn decode_kind(tag: u8) -> Result<NodeKind, PlanDecodeError> {
    match tag {
        1 => Ok(NodeKind::File),
        2 => Ok(NodeKind::Directory),
        3 => Ok(NodeKind::Symlink),
        _ => Err(PlanDecodeError::Tag),
    }
}

fn plan_digest(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"vsh\0commit-plan-v1\0");
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], PlanDecodeError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(PlanDecodeError::Truncated)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(PlanDecodeError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], PlanDecodeError> {
        self.take(N)?
            .try_into()
            .map_err(|_| PlanDecodeError::Truncated)
    }

    fn u8(&mut self) -> Result<u8, PlanDecodeError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, PlanDecodeError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    fn u64(&mut self) -> Result<u64, PlanDecodeError> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    fn i128(&mut self) -> Result<i128, PlanDecodeError> {
        Ok(i128::from_le_bytes(self.array()?))
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

/// Durable commit-plan decoding failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanDecodeError {
    /// Input ended before a complete value.
    Truncated,
    /// Plan checksum does not match its bytes.
    Checksum,
    /// Plan header is unknown.
    Magic,
    /// A semantic tag is unknown.
    Tag,
    /// A path is not UTF-8.
    Utf8,
    /// A path violates [`VPath`] rules.
    Path,
    /// A node-state encoding is inconsistent.
    State,
    /// Decoded counts or path lengths exceed configured bounds.
    Limit,
    /// Bytes remain after the exact plan payload.
    TrailingBytes,
}

impl fmt::Display for PlanDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Truncated => "commit plan is truncated",
            Self::Checksum => "commit plan checksum mismatch",
            Self::Magic => "commit plan has an unknown format",
            Self::Tag => "commit plan contains an unknown tag",
            Self::Utf8 => "commit plan path is not UTF-8",
            Self::Path => "commit plan path is invalid",
            Self::State => "commit plan node state is invalid",
            Self::Limit => "commit plan exceeds configured bounds",
            Self::TrailingBytes => "commit plan contains trailing bytes",
        })
    }
}

impl Error for PlanDecodeError {}
