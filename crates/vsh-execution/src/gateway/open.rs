use vsh_policy::AccessKind;
use vsh_types::{NodeKind, VPath};
use vsh_vfs::VfsError;

use super::{FsGateway, GatewayError};

/// Filesystem preparation needed by a guest-owned file handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenMode {
    /// Existing regular file, read only.
    Read,
    /// Existing regular file, readable and writable.
    ReadUpdate,
    /// Create or truncate, write only.
    Write,
    /// Create or truncate, readable and writable.
    WriteUpdate,
    /// Preserve an existing file or create an empty file, write only.
    Append,
    /// Preserve an existing file or create an empty file, readable and writable.
    AppendUpdate,
}

impl FsGateway<'_> {
    /// Authorize and prepare virtual state before a guest creates its own handle.
    ///
    /// No host descriptor or raw filesystem reference is returned. Later reads and
    /// writes must still pass through the gateway; preparation grants no cached access.
    ///
    /// # Errors
    /// Returns capability, type/parent or storage errors.
    pub fn prepare_open(&mut self, path: &VPath, mode: OpenMode) -> Result<(), GatewayError> {
        let accesses: &[AccessKind] = match mode {
            OpenMode::Read => &[AccessKind::ContentRead],
            OpenMode::ReadUpdate | OpenMode::WriteUpdate | OpenMode::AppendUpdate => &[
                AccessKind::ContentRead,
                AccessKind::Create,
                AccessKind::Modify,
            ],
            OpenMode::Write | OpenMode::Append => &[AccessKind::Create, AccessKind::Modify],
        };
        self.authorize(path, accesses)?;
        match mode {
            OpenMode::Read | OpenMode::ReadUpdate => {
                let state = self.apply(|filesystem| filesystem.metadata(path))?;
                if state.kind() != NodeKind::File {
                    return Err(VfsError::NotFile {
                        path: path.clone(),
                        actual: state.kind(),
                    }
                    .into());
                }
            }
            OpenMode::Write | OpenMode::WriteUpdate => {
                self.apply(|filesystem| filesystem.write(path, &[]))?;
            }
            OpenMode::Append | OpenMode::AppendUpdate => {
                match self.apply(|filesystem| filesystem.metadata(path)) {
                    Ok(state) if state.kind() == NodeKind::File => {}
                    Ok(state) => {
                        return Err(VfsError::NotFile {
                            path: path.clone(),
                            actual: state.kind(),
                        }
                        .into());
                    }
                    Err(GatewayError::Filesystem(source))
                        if matches!(*source, VfsError::NotFound { .. }) =>
                    {
                        self.apply(|filesystem| filesystem.write(path, &[]))?;
                    }
                    Err(source) => return Err(source),
                }
            }
        }
        Ok(())
    }
}
