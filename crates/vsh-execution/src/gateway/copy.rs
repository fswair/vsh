use vsh_policy::AccessKind;
use vsh_types::{NodeKind, NodeState, VPath};
use vsh_vfs::{EvidenceBuffer, VfsError};

use super::{FsGateway, GatewayError};

enum CopyEntry {
    Directory { destination: VPath, mode: u32 },
    File { destination: VPath, bytes: Vec<u8> },
}

impl FsGateway<'_> {
    /// Copy a regular file or preflighted directory tree within the active snapshot.
    ///
    /// Existing Monty semantics are preserved: files copy content into the destination
    /// mode; new directories retain source modes. Directory merging and symlink copy
    /// are unsupported. No tree mutation occurs until traversal and payload preflight
    /// succeeds; read/evidence work performed during preflight is not refunded.
    ///
    /// # Errors
    /// Returns invalid-operation, policy, traversal/byte limit or filesystem errors.
    pub fn copy(
        &mut self,
        source: &VPath,
        destination: &VPath,
        recursive: bool,
        overwrite: bool,
    ) -> Result<(), GatewayError> {
        self.authorize(source, &[AccessKind::MetadataRead])?;
        self.authorize(destination, &[AccessKind::Create, AccessKind::Modify])?;
        if destination.is_within(source) {
            return Err(GatewayError::InvalidOperation {
                reason: "destination cannot be inside its source",
                path: None,
            });
        }
        let parent = destination.parent().ok_or(VfsError::RootMutation)?;
        let state = self.apply(|filesystem| filesystem.metadata(&parent))?;
        if state.kind() != NodeKind::Directory {
            return Err(VfsError::NotDirectory {
                path: parent,
                actual: state.kind(),
            }
            .into());
        }
        let state = self.apply(|filesystem| filesystem.metadata(source))?;
        match state.kind() {
            NodeKind::File => self.copy_file(source, destination, state, overwrite),
            NodeKind::Directory if recursive => {
                self.copy_tree(source, destination, state, overwrite)
            }
            NodeKind::Directory => Err(GatewayError::InvalidOperation {
                reason: "source is a directory; pass recursive=True",
                path: None,
            }),
            NodeKind::Symlink => Err(GatewayError::InvalidOperation {
                reason: "does not copy symbolic links",
                path: None,
            }),
        }
    }

    fn copy_file(
        &mut self,
        source: &VPath,
        destination: &VPath,
        state: NodeState,
        overwrite: bool,
    ) -> Result<(), GatewayError> {
        match self.apply(|filesystem| filesystem.metadata(destination)) {
            Ok(_) if !overwrite => {
                return Err(VfsError::AlreadyExists {
                    path: destination.clone(),
                }
                .into());
            }
            Ok(state) if state.kind() == NodeKind::Directory => {
                return Err(VfsError::IsDirectory {
                    path: destination.clone(),
                }
                .into());
            }
            Ok(_) => {}
            Err(GatewayError::Filesystem(source))
                if matches!(*source, VfsError::NotFound { .. }) => {}
            Err(source) => return Err(source),
        }
        self.authorize(source, &[AccessKind::ContentRead])?;
        self.budget.charge_read(state.size())?;
        let bytes = self.apply(|filesystem| filesystem.read(source))?;
        self.budget.charge_write(bytes.len())?;
        self.apply(|filesystem| filesystem.write(destination, &bytes))
    }

    fn copy_tree(
        &mut self,
        source: &VPath,
        destination: &VPath,
        state: NodeState,
        overwrite: bool,
    ) -> Result<(), GatewayError> {
        match self.apply(|filesystem| filesystem.metadata(destination)) {
            Ok(_) if !overwrite => {
                return Err(VfsError::AlreadyExists {
                    path: destination.clone(),
                }
                .into());
            }
            Ok(_) => {
                return Err(GatewayError::InvalidOperation {
                    reason: "cannot merge or overwrite a directory tree",
                    path: None,
                });
            }
            Err(GatewayError::Filesystem(source))
                if matches!(*source, VfsError::NotFound { .. }) => {}
            Err(source) => return Err(source),
        }
        let mut buffer = EvidenceBuffer::default();
        self.reserve_buffer::<CopyEntry>(&mut buffer, destination.as_str().len())?;
        self.reserve_buffer::<VPath>(&mut buffer, source.as_str().len())?;
        let mut entries = vec![CopyEntry::Directory {
            destination: destination.clone(),
            mode: state.mode(),
        }];
        let mut pending = vec![source.clone()];
        while let Some(directory) = pending.pop() {
            self.authorize(&directory, &[AccessKind::DirectoryRead])?;
            let children = self.list_authorized_children(&directory)?;
            for child in children {
                self.authorize(&child, &[AccessKind::MetadataRead])?;
                let state = self.apply(|filesystem| filesystem.metadata(&child))?;
                // The source-plus-prefix length is a conservative rebase bound.
                // Reserve before constructing a long owned destination per child.
                let target_bytes = child
                    .as_str()
                    .len()
                    .saturating_add(destination.as_str().len())
                    .saturating_add(1);
                self.reserve_buffer::<CopyEntry>(&mut buffer, target_bytes)?;
                let target = child
                    .rebase(source, destination)
                    .map_err(VfsError::from)?
                    .ok_or_else(|| VfsError::InvalidRename {
                        from: child.clone(),
                        to: destination.clone(),
                    })?;
                self.authorize(&target, &[AccessKind::Create, AccessKind::Modify])?;
                match state.kind() {
                    NodeKind::Directory => {
                        self.reserve_buffer::<VPath>(&mut buffer, child.as_str().len())?;
                        entries.push(CopyEntry::Directory {
                            destination: target,
                            mode: state.mode(),
                        });
                        pending.push(child);
                    }
                    NodeKind::File => {
                        self.authorize(&child, &[AccessKind::ContentRead])?;
                        self.budget.charge_read(state.size())?;
                        let bytes = self.apply(|filesystem| filesystem.read(&child))?;
                        self.budget.charge_write(bytes.len())?;
                        entries.push(CopyEntry::File {
                            destination: target,
                            bytes,
                        });
                    }
                    NodeKind::Symlink => {
                        return Err(GatewayError::InvalidOperation {
                            reason: "does not copy symbolic link",
                            path: Some(child),
                        });
                    }
                }
            }
        }
        entries.sort_by_key(|entry| match entry {
            CopyEntry::Directory { destination, .. } | CopyEntry::File { destination, .. } => {
                destination.as_str().matches('/').count()
            }
        });
        for entry in entries {
            match entry {
                CopyEntry::Directory { destination, mode } => {
                    self.apply(|filesystem| filesystem.mkdir(&destination, mode))?;
                }
                CopyEntry::File { destination, bytes } => {
                    self.apply(|filesystem| filesystem.write(&destination, &bytes))?;
                }
            }
        }
        Ok(())
    }
}
