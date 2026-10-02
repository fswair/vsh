use vsh_policy::AccessKind;
use vsh_types::{NodeKind, NodeState, VPath};
use vsh_vfs::{EvidenceBuffer, VfsError};

use super::{FsGateway, GatewayError};

/// One currently observed node during a policy-filtered traversal.
///
/// The visitor can read this node but cannot mutate the gateway. Its cached size
/// remains valid until the visitor returns, so a content read need not repeat the
/// metadata operation or give an adapter an unbounded/unverified read primitive.
pub struct ObservedEntry<'a, 'fs> {
    path: &'a VPath,
    state: NodeState,
    gateway: &'a mut FsGateway<'fs>,
}

impl ObservedEntry<'_, '_> {
    /// Return the normalized path already authorized for metadata access.
    #[must_use]
    pub const fn path(&self) -> &VPath {
        self.path
    }

    /// Return the metadata observed by the traversal.
    #[must_use]
    pub const fn state(&self) -> NodeState {
        self.state
    }

    /// Read this regular file with separate content authorization and accounting.
    ///
    /// # Errors
    /// Returns a content denial, byte limit or filesystem failure.
    pub fn read(&mut self) -> Result<Vec<u8>, GatewayError> {
        if self.state.kind() != NodeKind::File {
            return Err(VfsError::NotFile {
                path: self.path.clone(),
                actual: self.state.kind(),
            }
            .into());
        }
        self.gateway
            .authorize(self.path, &[AccessKind::ContentRead])?;
        self.gateway.budget.charge_read(self.state.size())?;
        self.gateway.apply(|filesystem| filesystem.read(self.path))
    }
}

impl<'fs> FsGateway<'fs> {
    /// Observe one node without allowing a mutation between metadata and content.
    ///
    /// # Errors
    /// Returns a metadata denial or filesystem error.
    pub fn observe<'a>(
        &'a mut self,
        path: &'a VPath,
    ) -> Result<ObservedEntry<'a, 'fs>, GatewayError> {
        let state = self.metadata(path)?;
        Ok(ObservedEntry {
            path,
            state,
            gateway: self,
        })
    }

    /// Visit visible descendants in canonical depth-first order, stopping on `false`.
    ///
    /// Hidden metadata paths and non-enumerable directories are skipped, preserving
    /// Monty's visible traversal contract. Listings still charge all physical entries
    /// and record their full dependency. This is not a mutation traversal: copy,
    /// rename and deletion preflight every affected path instead of skipping it.
    ///
    /// # Errors
    /// Returns root, traversal or visitor failures without erasing performed work.
    pub fn walk_visible<E>(
        &mut self,
        root: &VPath,
        mut visit: impl FnMut(ObservedEntry<'_, '_>) -> Result<bool, E>,
    ) -> Result<(), E>
    where
        E: From<GatewayError>,
    {
        let state = self.metadata(root)?;
        if state.kind() != NodeKind::Directory {
            return Err(GatewayError::from(VfsError::NotDirectory {
                path: root.clone(),
                actual: state.kind(),
            })
            .into());
        }
        self.authorize(root, &[AccessKind::DirectoryRead])?;
        let children = self.list_authorized_children(root)?;
        let mut buffer = EvidenceBuffer::default();
        for child in &children {
            self.reserve_buffer::<VPath>(&mut buffer, child.as_str().len())?;
        }
        let mut pending = children.into_iter().rev().collect::<Vec<_>>();
        while let Some(path) = pending.pop() {
            match self.authorize(&path, &[AccessKind::MetadataRead]) {
                Ok(()) => {}
                Err(GatewayError::Policy(_)) => continue,
                Err(source) => return Err(source.into()),
            }
            let state = self.apply(|filesystem| filesystem.metadata(&path))?;
            if !visit(ObservedEntry {
                path: &path,
                state,
                gateway: self,
            })? {
                return Ok(());
            }
            if state.kind() == NodeKind::Directory
                && self
                    .policy
                    .authorize(&path, AccessKind::DirectoryRead)
                    .is_ok()
            {
                let children = self.list_authorized_children(&path)?;
                for child in &children {
                    self.reserve_buffer::<VPath>(&mut buffer, child.as_str().len())?;
                }
                pending.extend(children.into_iter().rev());
            }
        }
        Ok(())
    }
}
