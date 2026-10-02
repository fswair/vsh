use std::error::Error;
use std::fmt;

use vsh_policy::{AccessKind, CallPolicy, DeniedAccess};
use vsh_types::{NodeKind, NodeState, VPath};
use vsh_vfs::{
    EffectOrigin, EvidenceBuffer, EvidenceLimitExceeded, EvidenceLimits, VfsError, VirtualFs,
};

use crate::{ExecutionBudget, ExecutionLimitExceeded, ExecutionLimits};

mod copy;
mod open;
mod walk;

pub use open::OpenMode;
pub use walk::ObservedEntry;

/// Failure at the guest-independent filesystem authority boundary.
#[derive(Debug)]
pub enum GatewayError {
    /// A requested compound operation has no valid supported meaning.
    InvalidOperation {
        /// Backend-neutral explanation.
        reason: &'static str,
        /// Offending path, when the failure is path-specific.
        path: Option<VPath>,
    },
    /// A capability was rejected before filesystem access.
    Policy(Box<DeniedAccess>),
    /// Host-side work accounting rejected the operation.
    Limit(ExecutionLimitExceeded),
    /// Virtual filesystem semantics or snapshot integrity failed.
    Filesystem(Box<VfsError>),
}

impl From<VfsError> for GatewayError {
    fn from(source: VfsError) -> Self {
        match source {
            VfsError::EvidenceLimit(EvidenceLimitExceeded::Records { limit, attempted }) => {
                Self::Limit(ExecutionLimitExceeded::EvidenceRecords { limit, attempted })
            }
            VfsError::EvidenceLimit(EvidenceLimitExceeded::Bytes { limit, attempted }) => {
                Self::Limit(ExecutionLimitExceeded::EvidenceBytes { limit, attempted })
            }
            source => Self::Filesystem(Box::new(source)),
        }
    }
}

impl From<ExecutionLimitExceeded> for GatewayError {
    fn from(source: ExecutionLimitExceeded) -> Self {
        Self::Limit(source)
    }
}

impl From<DeniedAccess> for GatewayError {
    fn from(source: DeniedAccess) -> Self {
        Self::Policy(Box::new(source))
    }
}

impl fmt::Display for GatewayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOperation { reason, path } => {
                formatter.write_str(reason)?;
                if let Some(path) = path {
                    write!(formatter, " {:?}", path.as_str())?;
                }
                Ok(())
            }
            Self::Policy(source) => write!(formatter, "filesystem capability denied: {source:?}"),
            Self::Limit(source) => source.fmt(formatter),
            Self::Filesystem(source) => source.fmt(formatter),
        }
    }
}

impl Error for GatewayError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::InvalidOperation { .. } | Self::Policy(_) => None,
            Self::Limit(source) => Some(source),
            Self::Filesystem(source) => Some(source),
        }
    }
}

/// Validate an incoming path's byte length before normalization or allocation.
///
/// # Errors
/// Returns a path limit error when `raw` exceeds the configured cap.
pub fn check_path_bytes(raw: &str, limits: ExecutionLimits) -> Result<(), ExecutionLimitExceeded> {
    let attempted = u64::try_from(raw.len()).unwrap_or(u64::MAX);
    let limit = u64::try_from(limits.max_path_bytes).unwrap_or(u64::MAX);
    if attempted > limit {
        Err(ExecutionLimitExceeded::PathBytes { limit, attempted })
    } else {
        Ok(())
    }
}

/// Preflight all required capabilities without observing the filesystem.
///
/// # Errors
/// Returns the first policy denial in the supplied capability order.
pub fn authorize_path(
    policy: &CallPolicy,
    path: &VPath,
    accesses: &[AccessKind],
) -> Result<(), DeniedAccess> {
    for access in accesses {
        policy.authorize(path, *access)?;
    }
    Ok(())
}

/// Retain a caught policy denial under the same active parent-evidence budget.
///
/// Ignored traversal-filter denials do not call this function or consume records.
///
/// # Errors
/// Returns a terminal evidence resource failure before vector growth.
pub fn retain_denial(
    filesystem: &mut VirtualFs,
    limits: ExecutionLimits,
    denied_accesses: &mut Vec<DeniedAccess>,
    denial: DeniedAccess,
) -> Result<(), GatewayError> {
    filesystem.limit_evidence(EvidenceLimits {
        max_records: limits.max_evidence_records,
        max_bytes: limits.max_evidence_bytes,
    });
    let bytes = denial
        .path
        .as_str()
        .len()
        .checked_add(denial.rule.len())
        .and_then(|value| value.checked_add(2 * size_of::<DeniedAccess>()))
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(u64::MAX);
    filesystem.reserve_adapter_evidence(bytes)?;
    if denied_accesses.len() == denied_accesses.capacity() {
        denied_accesses
            .try_reserve_exact(denied_accesses.capacity().max(1))
            .map_err(|_| filesystem.stop_evidence(EvidenceLimitExceeded::Allocation))?;
    }
    denied_accesses.push(denial);
    Ok(())
}

/// Borrowed filesystem gateway; owns no snapshot and exposes no mutable VFS handle.
///
/// An adapter charges one request on the shared budget before dispatch. Composite
/// operations charge their actual bytes and listings, not fictitious extra guest
/// requests. The adapter retains policy failures even if its guest catches an error.
pub struct FsGateway<'a> {
    filesystem: &'a mut VirtualFs,
    policy: &'a CallPolicy,
    budget: &'a mut ExecutionBudget,
    origin: EffectOrigin,
}

impl<'a> FsGateway<'a> {
    /// Borrow the active transaction with an explicit effect origin.
    #[must_use]
    pub fn new(
        filesystem: &'a mut VirtualFs,
        policy: &'a CallPolicy,
        budget: &'a mut ExecutionBudget,
        origin: EffectOrigin,
    ) -> Self {
        filesystem.limit_evidence(EvidenceLimits {
            max_records: budget.limits().max_evidence_records,
            max_bytes: budget.limits().max_evidence_bytes,
        });
        Self {
            filesystem,
            policy,
            budget,
            origin,
        }
    }

    /// Observe metadata, including the VFS missing-path dependency on failure.
    ///
    /// # Errors
    /// Returns a policy denial or the underlying filesystem error.
    pub fn metadata(&mut self, path: &VPath) -> Result<NodeState, GatewayError> {
        self.authorize(path, &[AccessKind::MetadataRead])?;
        self.apply(|filesystem| filesystem.metadata(path))
    }

    /// Read a regular file after authorizing and charging materialized bytes.
    ///
    /// # Errors
    /// Returns a denial, byte limit, missing/non-file or integrity error.
    pub fn read(&mut self, path: &VPath) -> Result<Vec<u8>, GatewayError> {
        self.authorize(path, &[AccessKind::ContentRead])?;
        let state = self.apply(|filesystem| filesystem.metadata(path))?;
        if state.kind() != NodeKind::File {
            return Err(VfsError::NotFile {
                path: path.clone(),
                actual: state.kind(),
            }
            .into());
        }
        self.budget.charge_read(state.size())?;
        self.apply(|filesystem| filesystem.read(path))
    }

    /// Read a symbolic link's opaque bytes without following it.
    ///
    /// # Errors
    /// Returns a denial, byte limit, missing/non-link or integrity error.
    pub fn read_link(&mut self, path: &VPath) -> Result<Vec<u8>, GatewayError> {
        self.authorize(path, &[AccessKind::ContentRead])?;
        let state = self.apply(|filesystem| filesystem.metadata(path))?;
        if state.kind() != NodeKind::Symlink {
            return Err(VfsError::NotSymlink {
                path: path.clone(),
                actual: state.kind(),
            }
            .into());
        }
        self.budget.charge_read(state.size())?;
        self.apply(|filesystem| filesystem.read_link(path))
    }

    /// Create or replace regular-file bytes; failed work is not refunded.
    ///
    /// # Errors
    /// Returns a denial, byte limit, type/parent or storage error.
    pub fn write(&mut self, path: &VPath, bytes: &[u8]) -> Result<(), GatewayError> {
        self.budget.charge_write(bytes.len())?;
        self.authorize(path, &[AccessKind::Create, AccessKind::Modify])?;
        self.apply(|filesystem| filesystem.write(path, bytes))
    }

    /// Append to an existing file or create a missing file.
    ///
    /// Existing-file reconstruction is charged as a read, matching VSH semantics.
    ///
    /// # Errors
    /// Returns a denial, read/write limit, type/parent or storage error.
    pub fn append(&mut self, path: &VPath, bytes: &[u8]) -> Result<(), GatewayError> {
        self.budget.charge_write(bytes.len())?;
        self.authorize(path, &[AccessKind::Create, AccessKind::Modify])?;
        match self.apply(|filesystem| filesystem.metadata(path)) {
            Ok(state) if state.kind() == NodeKind::File => {
                self.budget.charge_read(state.size())?;
                self.apply(|filesystem| filesystem.append(path, bytes))
            }
            Ok(state) => Err(VfsError::NotFile {
                path: path.clone(),
                actual: state.kind(),
            }
            .into()),
            Err(GatewayError::Filesystem(source))
                if matches!(*source, VfsError::NotFound { .. }) =>
            {
                self.apply(|filesystem| filesystem.write(path, bytes))
            }
            Err(source) => Err(source),
        }
    }

    /// List visible child paths; hidden entries still consume listing work.
    ///
    /// # Errors
    /// Returns a directory denial, entry limit or filesystem error.
    pub fn read_dir(&mut self, path: &VPath) -> Result<Vec<VPath>, GatewayError> {
        self.authorize(path, &[AccessKind::DirectoryRead])?;
        let mut children = self.list_authorized_children(path)?;
        children.retain(|child| {
            self.policy
                .authorize(child, AccessKind::MetadataRead)
                .is_ok()
        });
        Ok(children)
    }

    fn list_authorized_children(&mut self, path: &VPath) -> Result<Vec<VPath>, GatewayError> {
        let used = self.budget.stats().directory_entries;
        let limit = self.budget.limits().max_directory_entries;
        let remaining = usize::try_from(limit.saturating_sub(used)).unwrap_or(usize::MAX);
        let children =
            match self.apply(|filesystem| filesystem.read_dir_with_limit(path, remaining)) {
                Err(GatewayError::Filesystem(source)) => match *source {
                    VfsError::DirectoryEntryLimit { attempted, .. } => {
                        return Err(ExecutionLimitExceeded::DirectoryEntries {
                            limit,
                            attempted: used
                                .saturating_add(u64::try_from(attempted).unwrap_or(u64::MAX)),
                        }
                        .into());
                    }
                    source => return Err(source.into()),
                },
                result => result?,
            };
        self.budget.charge_directory_entries(children.len())?;
        Ok(children)
    }

    /// Create one directory after authorizing its creation.
    ///
    /// # Errors
    /// Returns a creation denial or filesystem parent/type error.
    pub fn mkdir(&mut self, path: &VPath, mode: u32) -> Result<(), GatewayError> {
        self.authorize(path, &[AccessKind::Create])?;
        self.apply(|filesystem| filesystem.mkdir(path, mode))
    }

    /// Prepare a directory creation with explicit ancestor and existing-path rules.
    ///
    /// Every candidate ancestor is authorized before creation, preserving Monty's
    /// conservative Create/Modify contract even for existing ancestors.
    ///
    /// # Errors
    /// Returns a denial or filesystem error before any forbidden ancestor is created.
    pub fn mkdir_options(
        &mut self,
        path: &VPath,
        mode: u32,
        parents: bool,
        exist_ok: bool,
    ) -> Result<(), GatewayError> {
        self.authorize(path, &[AccessKind::Create, AccessKind::Modify])?;
        if parents {
            let mut ancestor = path.parent();
            while let Some(candidate) = ancestor {
                if candidate.is_root() {
                    break;
                }
                self.authorize(&candidate, &[AccessKind::Create, AccessKind::Modify])?;
                ancestor = candidate.parent();
            }
        }
        match self.apply(|filesystem| filesystem.metadata(path)) {
            Ok(state) if exist_ok && state.kind() == NodeKind::Directory => return Ok(()),
            Ok(_) => return Err(VfsError::AlreadyExists { path: path.clone() }.into()),
            Err(GatewayError::Filesystem(source))
                if matches!(*source, VfsError::NotFound { .. }) => {}
            Err(source) => return Err(source),
        }
        if !parents {
            return self.apply(|filesystem| filesystem.mkdir(path, mode));
        }
        let mut buffer = EvidenceBuffer::default();
        self.reserve_buffer::<VPath>(&mut buffer, path.as_str().len())?;
        let mut missing = vec![path.clone()];
        let mut cursor = path.parent();
        while let Some(parent) = cursor {
            match self.apply(|filesystem| filesystem.metadata(&parent)) {
                Ok(state) if state.kind() == NodeKind::Directory => break,
                Ok(state) => {
                    return Err(VfsError::NotDirectory {
                        path: parent,
                        actual: state.kind(),
                    }
                    .into());
                }
                Err(GatewayError::Filesystem(source))
                    if matches!(*source, VfsError::NotFound { .. }) =>
                {
                    cursor = parent.parent();
                    self.reserve_buffer::<VPath>(&mut buffer, parent.as_str().len())?;
                    missing.push(parent);
                }
                Err(source) => return Err(source),
            }
        }
        for directory in missing.iter().rev() {
            self.apply(|filesystem| filesystem.mkdir(directory, mode))?;
        }
        Ok(())
    }

    /// Change ordinary permission bits with metadata and modification authority.
    ///
    /// # Errors
    /// Returns a denial, missing node or unsupported mode/link error.
    pub fn set_mode(&mut self, path: &VPath, mode: u32) -> Result<(), GatewayError> {
        self.authorize(path, &[AccessKind::MetadataRead, AccessKind::Modify])?;
        self.apply(|filesystem| filesystem.set_mode(path, mode))
    }

    /// Remove a non-directory node.
    ///
    /// # Errors
    /// Returns a deletion denial or filesystem error.
    pub fn unlink(&mut self, path: &VPath) -> Result<(), GatewayError> {
        self.authorize(path, &[AccessKind::Delete])?;
        self.apply(|filesystem| filesystem.unlink(path))
    }

    /// Remove an empty directory.
    ///
    /// # Errors
    /// Returns a deletion denial or filesystem error.
    pub fn rmdir(&mut self, path: &VPath) -> Result<(), GatewayError> {
        self.authorize(path, &[AccessKind::Delete, AccessKind::DirectoryRead])?;
        if path.is_root() {
            return Err(VfsError::RootMutation.into());
        }
        if !self.list_authorized_children(path)?.is_empty() {
            return Err(VfsError::DirectoryNotEmpty { path: path.clone() }.into());
        }
        self.apply(|filesystem| filesystem.rmdir(path))
    }

    /// Remove a file/link, empty directory or explicitly recursive directory tree.
    ///
    /// # Errors
    /// Returns a denial, traversal limit or filesystem failure. `missing_ok` only
    /// handles ordinary absence, never a policy denial or integrity error.
    pub fn remove(
        &mut self,
        path: &VPath,
        recursive: bool,
        missing_ok: bool,
    ) -> Result<(), GatewayError> {
        self.authorize(path, &[AccessKind::Delete])?;
        let state = match self.apply(|filesystem| filesystem.metadata(path)) {
            Ok(state) => state,
            Err(GatewayError::Filesystem(source))
                if missing_ok && matches!(*source, VfsError::NotFound { .. }) =>
            {
                return Ok(());
            }
            Err(source) => return Err(source),
        };
        match state.kind() {
            NodeKind::File | NodeKind::Symlink => self.apply(|filesystem| filesystem.unlink(path)),
            NodeKind::Directory if recursive => self.remove_tree(path),
            NodeKind::Directory => self.rmdir(path),
        }
    }

    /// Remove a subtree only after authorizing every affected child.
    ///
    /// # Errors
    /// Returns a denial, traversal limit or filesystem error without partial deletion.
    pub fn remove_tree(&mut self, root: &VPath) -> Result<(), GatewayError> {
        let mut buffer = EvidenceBuffer::default();
        self.reserve_buffer::<VPath>(&mut buffer, root.as_str().len())?;
        let mut pending = vec![root.clone()];
        while let Some(directory) = pending.pop() {
            self.authorize(&directory, &[AccessKind::Delete, AccessKind::DirectoryRead])?;
            let children = self.list_authorized_children(&directory)?;
            for child in children {
                self.authorize(&child, &[AccessKind::Delete])?;
                let state = self.apply(|filesystem| filesystem.metadata(&child))?;
                if state.kind() == NodeKind::Directory {
                    self.reserve_buffer::<VPath>(&mut buffer, child.as_str().len())?;
                    pending.push(child);
                }
            }
        }
        self.apply(|filesystem| filesystem.remove_tree(root))
    }

    /// Move a subtree after checking source and rebased destination capabilities.
    ///
    /// # Errors
    /// Returns a denial, traversal/path limit or filesystem error before mutation.
    pub fn rename(&mut self, source: &VPath, destination: &VPath) -> Result<(), GatewayError> {
        self.authorize(source, &[AccessKind::RenameSource])?;
        self.authorize(destination, &[AccessKind::RenameDestination])?;
        if source.is_root() || destination.is_root() {
            return Err(VfsError::RootMutation.into());
        }
        if source != destination && (source.is_within(destination) || destination.is_within(source))
        {
            return Err(VfsError::InvalidRename {
                from: source.clone(),
                to: destination.clone(),
            }
            .into());
        }
        if source != destination {
            let state = self.apply(|filesystem| filesystem.metadata(source))?;
            let mut buffer = EvidenceBuffer::default();
            let mut pending = Vec::new();
            if state.kind() == NodeKind::Directory {
                self.reserve_buffer::<VPath>(&mut buffer, source.as_str().len())?;
                pending.push(source.clone());
            }
            while let Some(directory) = pending.pop() {
                self.authorize(&directory, &[AccessKind::DirectoryRead])?;
                let children = self.list_authorized_children(&directory)?;
                for child in children {
                    self.authorize(&child, &[AccessKind::RenameSource])?;
                    let target = child
                        .rebase(source, destination)
                        .map_err(VfsError::from)?
                        .ok_or_else(|| VfsError::InvalidRename {
                            from: child.clone(),
                            to: destination.clone(),
                        })?;
                    self.authorize(&target, &[AccessKind::RenameDestination])?;
                    let state = self.apply(|filesystem| filesystem.metadata(&child))?;
                    if state.kind() == NodeKind::Directory {
                        self.reserve_buffer::<VPath>(&mut buffer, child.as_str().len())?;
                        pending.push(child);
                    }
                }
            }
        }
        self.apply(|filesystem| filesystem.rename(source, destination))
    }

    fn reserve_buffer<T>(
        &mut self,
        buffer: &mut EvidenceBuffer,
        path_bytes: usize,
    ) -> Result<(), GatewayError> {
        // Four slots cover Vec's initial small capacity and geometric growth.
        let bytes = path_bytes
            .checked_add(4 * size_of::<T>())
            .and_then(|value| u64::try_from(value).ok())
            .unwrap_or(u64::MAX);
        self.filesystem.reserve_evidence_buffer(buffer, bytes)?;
        Ok(())
    }

    fn authorize(&self, path: &VPath, accesses: &[AccessKind]) -> Result<(), GatewayError> {
        self.filesystem.check_evidence()?;
        check_path_bytes(path.as_str(), self.budget.limits())?;
        authorize_path(self.policy, path, accesses).map_err(GatewayError::from)
    }

    fn apply<T>(
        &mut self,
        operation: impl FnOnce(&mut VirtualFs) -> Result<T, VfsError>,
    ) -> Result<T, GatewayError> {
        self.filesystem.check_evidence()?;
        self.filesystem
            .with_effect_origin(self.origin, operation)
            .map_err(GatewayError::from)
    }
}
