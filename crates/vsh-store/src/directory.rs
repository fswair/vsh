use std::error::Error;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cap_std::ambient_authority;
use cap_std::fs::{Dir, Metadata, MetadataExt, OpenOptions};

const RUNTIME_DIRECTORY: &str = ".vsh-runtime";
const DATA_DIRECTORY: &str = "data";

/// A pinned capability for VSH's durable data directory.
///
/// Runtime-owned files are opened relative to this handle, so replacing an
/// ambient ancestor with a symlink cannot redirect later blob or state-store I/O.
#[derive(Clone)]
pub struct DataDirectory {
    path: Arc<PathBuf>,
    directory: Arc<Dir>,
}

impl DataDirectory {
    /// Open a caller-selected, trusted data directory with ambient authority.
    ///
    /// The final component must be a real directory rather than a symbolic link.
    /// Runtime callers should keep this directory disjoint from the untrusted
    /// workspace; [`Self::open_workspace`] is the safe constructor for the default
    /// workspace-local location.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created, pinned, or verified.
    pub fn open_trusted(path: impl AsRef<Path>) -> Result<Self, DataDirectoryError> {
        let requested = path.as_ref();
        let path = std::path::absolute(requested).map_err(|source| {
            DataDirectoryError::io("resolve trusted data directory", requested, source)
        })?;
        Dir::create_ambient_dir_all(&path, ambient_authority()).map_err(|source| {
            DataDirectoryError::io("create trusted data directory", &path, source)
        })?;
        let before = fs::symlink_metadata(&path).map_err(|source| {
            DataDirectoryError::io("inspect trusted data directory", &path, source)
        })?;
        if !before.is_dir() || before.file_type().is_symlink() {
            return Err(DataDirectoryError::not_real(&path));
        }
        let directory = Dir::open_ambient_dir(&path, ambient_authority()).map_err(|source| {
            DataDirectoryError::io("open trusted data directory", &path, source)
        })?;
        let opened = directory.dir_metadata().map_err(|source| {
            DataDirectoryError::io("inspect opened data directory", &path, source)
        })?;
        let after = fs::symlink_metadata(&path).map_err(|source| {
            DataDirectoryError::io("reinspect trusted data directory", &path, source)
        })?;
        if !after.is_dir() || after.file_type().is_symlink() || !opened_matches_std(&opened, &after)
        {
            return Err(DataDirectoryError::unstable(&path));
        }
        let canonical_path = fs::canonicalize(&path).map_err(|source| {
            DataDirectoryError::io("canonicalize trusted data directory", &path, source)
        })?;
        let canonical = fs::symlink_metadata(&canonical_path).map_err(|source| {
            DataDirectoryError::io(
                "inspect canonical trusted data directory",
                &canonical_path,
                source,
            )
        })?;
        let final_named = fs::symlink_metadata(&path).map_err(|source| {
            DataDirectoryError::io("finalize trusted data directory", &path, source)
        })?;
        if !canonical.is_dir()
            || canonical.file_type().is_symlink()
            || !opened_matches_std(&opened, &canonical)
            || !final_named.is_dir()
            || final_named.file_type().is_symlink()
            || !opened_matches_std(&opened, &final_named)
        {
            return Err(DataDirectoryError::unstable(&path));
        }
        sync_directory(&directory).map_err(|source| {
            DataDirectoryError::io("sync trusted data directory", &path, source)
        })?;
        Ok(Self::new(canonical_path, directory))
    }

    /// Open the protected `.vsh-runtime/data` directory below a workspace handle.
    ///
    /// Every child is created and verified relative to a pinned workspace
    /// capability. Pre-existing symlinks and directory-swap races therefore fail
    /// closed without writing outside the workspace root.
    ///
    /// # Errors
    ///
    /// Returns an error if the workspace cannot be opened or either protected
    /// component is not a stable real directory.
    pub fn open_workspace(workspace_root: impl AsRef<Path>) -> Result<Self, DataDirectoryError> {
        let requested = workspace_root.as_ref();
        let workspace_root = std::path::absolute(requested).map_err(|source| {
            DataDirectoryError::io("resolve workspace data capability", requested, source)
        })?;
        let root =
            Dir::open_ambient_dir(&workspace_root, ambient_authority()).map_err(|source| {
                DataDirectoryError::io("open workspace data capability", &workspace_root, source)
            })?;
        let runtime_path = workspace_root.join(RUNTIME_DIRECTORY);
        let runtime = open_or_create_real_dir(&root, RUNTIME_DIRECTORY).map_err(|source| {
            DataDirectoryError::io("open protected runtime directory", &runtime_path, source)
        })?;
        let data = Self::open_runtime_data(&runtime, &workspace_root)?;
        sync_directory(&runtime).map_err(|source| {
            DataDirectoryError::io("sync protected runtime directory", &runtime_path, source)
        })?;
        sync_directory(&root).map_err(|source| {
            DataDirectoryError::io("sync workspace root", &workspace_root, source)
        })?;
        Ok(data)
    }

    /// Open `.vsh-runtime/data` relative to an already pinned runtime directory.
    ///
    /// This constructor lets the trusted committer and durable stores share one
    /// runtime-directory identity instead of reopening an ambient path.
    ///
    /// # Errors
    ///
    /// Returns an error if `data` cannot be created, pinned, or synchronized.
    pub fn open_runtime_data(
        runtime: &Dir,
        workspace_root: impl AsRef<Path>,
    ) -> Result<Self, DataDirectoryError> {
        let requested = workspace_root.as_ref();
        let workspace_root = std::path::absolute(requested).map_err(|source| {
            DataDirectoryError::io("resolve protected data directory", requested, source)
        })?;
        let data_path = workspace_root.join(RUNTIME_DIRECTORY).join(DATA_DIRECTORY);
        let data = open_or_create_real_dir(runtime, DATA_DIRECTORY).map_err(|source| {
            DataDirectoryError::io("open protected data directory", &data_path, source)
        })?;
        sync_directory(&data).map_err(|source| {
            DataDirectoryError::io("sync protected data directory", &data_path, source)
        })?;
        sync_directory(runtime).map_err(|source| {
            DataDirectoryError::io(
                "sync protected data-directory parent",
                &workspace_root.join(RUNTIME_DIRECTORY),
                source,
            )
        })?;
        Ok(Self::new(data_path, data))
    }

    fn new(path: PathBuf, directory: Dir) -> Self {
        Self {
            path: Arc::new(path),
            directory: Arc::new(directory),
        }
    }

    /// Return the ambient path used only for diagnostics and backup tooling.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn directory(&self) -> &Dir {
        &self.directory
    }

    pub(crate) fn open_real_child(&self, name: &str) -> io::Result<Self> {
        let directory = open_or_create_real_dir(&self.directory, name)?;
        Ok(Self::new(self.path.join(name), directory))
    }
}

impl fmt::Debug for DataDirectory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DataDirectory")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl PartialEq for DataDirectory {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl Eq for DataDirectory {}

pub(crate) fn open_or_create_real_dir(parent: &Dir, name: &str) -> io::Result<Dir> {
    match parent.create_dir(name) {
        Ok(()) => {}
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
        Err(source) => return Err(source),
    }
    let before = parent.symlink_metadata(name)?;
    if !before.is_dir() || before.is_symlink() {
        return Err(not_real_directory_error());
    }
    let child = parent.open_dir(name)?;
    let opened = child.dir_metadata()?;
    let after = parent.symlink_metadata(name)?;
    if !after.is_dir() || after.is_symlink() || !metadata_identity_matches(&opened, &after) {
        return Err(unstable_directory_error());
    }
    Ok(child)
}

pub(crate) fn open_real_file(
    directory: &Dir,
    name: &str,
    options: &OpenOptions,
) -> io::Result<fs::File> {
    let file = directory.open_with(name, options)?.into_std();
    let opened = file.metadata()?;
    let named = directory.symlink_metadata(name)?;
    if !opened.is_file()
        || !named.is_file()
        || named.is_symlink()
        || !std_matches_cap(&opened, &named)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "internal VSH file is not a stable real file",
        ));
    }
    Ok(file)
}

#[cfg(not(windows))]
pub(crate) fn sync_directory(directory: &Dir) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.read(true);
    directory.open_with(".", &options)?.into_std().sync_all()
}

#[cfg(windows)]
pub(crate) fn sync_directory(_directory: &Dir) -> io::Result<()> {
    Ok(())
}

fn not_real_directory_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "internal VSH path is not a real directory",
    )
}

fn unstable_directory_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "internal VSH directory changed while it was being pinned",
    )
}

#[cfg(unix)]
fn metadata_identity_matches(left: &Metadata, right: &Metadata) -> bool {
    MetadataExt::dev(left) == MetadataExt::dev(right)
        && MetadataExt::ino(left) == MetadataExt::ino(right)
}

#[cfg(windows)]
fn metadata_identity_matches(left: &Metadata, right: &Metadata) -> bool {
    MetadataExt::volume_serial_number(left) == MetadataExt::volume_serial_number(right)
        && MetadataExt::file_index(left) == MetadataExt::file_index(right)
}

#[cfg(unix)]
fn std_matches_cap(opened: &fs::Metadata, named: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    opened.dev() == MetadataExt::dev(named) && opened.ino() == MetadataExt::ino(named)
}

#[cfg(windows)]
fn std_matches_cap(opened: &fs::Metadata, named: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    opened.volume_serial_number() == MetadataExt::volume_serial_number(named)
        && opened.file_index() == MetadataExt::file_index(named)
}

#[cfg(unix)]
fn opened_matches_std(opened: &Metadata, named: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    MetadataExt::dev(opened) == named.dev() && MetadataExt::ino(opened) == named.ino()
}

#[cfg(windows)]
fn opened_matches_std(opened: &Metadata, named: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt as _;

    MetadataExt::volume_serial_number(opened) == named.volume_serial_number()
        && MetadataExt::file_index(opened) == named.file_index()
}

#[cfg(not(any(unix, windows)))]
compile_error!("vsh-store currently supports Unix and Windows hosts");

/// Failure to create or pin a durable VSH data directory.
#[derive(Debug)]
pub struct DataDirectoryError {
    operation: &'static str,
    path: PathBuf,
    source: io::Error,
}

impl DataDirectoryError {
    fn io(operation: &'static str, path: &Path, source: io::Error) -> Self {
        Self {
            operation,
            path: path.to_owned(),
            source,
        }
    }

    fn not_real(path: &Path) -> Self {
        Self::io(
            "verify trusted data directory",
            path,
            not_real_directory_error(),
        )
    }

    fn unstable(path: &Path) -> Self {
        Self::io(
            "verify trusted data directory",
            path,
            unstable_directory_error(),
        )
    }
}

impl fmt::Display for DataDirectoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at {}: {}",
            self.operation,
            self.path.display(),
            self.source
        )
    }
}

impl Error for DataDirectoryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "vsh-data-directory-test-{}-{sequence}-{name}",
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

    #[cfg(not(windows))]
    #[test]
    fn capability_directory_can_be_synchronized() {
        let workspace = TestDirectory::new("sync-capability");
        let directory = Dir::open_ambient_dir(workspace.path(), ambient_authority()).unwrap();

        sync_directory(&directory).unwrap();
    }

    #[test]
    fn workspace_data_directory_is_real_and_pinned() {
        let workspace = TestDirectory::new("pinned-workspace");
        let outside = TestDirectory::new("pinned-outside");
        let data = DataDirectory::open_workspace(workspace.path()).unwrap();

        assert_eq!(data.path(), workspace.path().join(".vsh-runtime/data"));
        assert_eq!(data, data.clone());
        assert!(format!("{data:?}").contains("DataDirectory"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let original = workspace.path().join(".vsh-runtime");
            let relocated = workspace.path().join(".vsh-runtime-relocated");
            fs::rename(&original, &relocated).unwrap();
            symlink(outside.path(), &original).unwrap();

            let child = data.open_real_child("still-pinned").unwrap();
            assert!(relocated.join("data/still-pinned").is_dir());
            assert!(!outside.path().join("data/still-pinned").exists());
            drop(child);
        }
    }

    #[cfg(unix)]
    #[test]
    fn workspace_runtime_symlink_is_rejected_before_external_write() {
        use std::os::unix::fs::symlink;

        let workspace = TestDirectory::new("symlink-workspace");
        let outside = TestDirectory::new("symlink-outside");
        symlink(outside.path(), workspace.path().join(".vsh-runtime")).unwrap();

        let error = DataDirectory::open_workspace(workspace.path()).unwrap_err();

        assert_eq!(
            error
                .source()
                .unwrap()
                .downcast_ref::<io::Error>()
                .unwrap()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert!(!outside.path().join("data").exists());
    }

    #[cfg(unix)]
    #[test]
    fn trusted_data_directory_rejects_a_final_symlink() {
        use std::os::unix::fs::symlink;

        let parent = TestDirectory::new("trusted-parent");
        let outside = TestDirectory::new("trusted-outside");
        let link = parent.path().join("data-link");
        symlink(outside.path(), &link).unwrap();

        let error = DataDirectory::open_trusted(&link).unwrap_err();
        assert!(error.to_string().contains("data-link"));
        assert!(error.source().is_some());
    }

    #[test]
    fn missing_workspace_and_non_directory_data_fail_with_typed_sources() {
        let parent = TestDirectory::new("invalid-components");
        let missing = parent.path().join("missing-workspace");
        let missing_error = DataDirectory::open_workspace(&missing).unwrap_err();
        assert!(missing_error.to_string().contains("workspace"));
        assert!(missing_error.source().is_some());

        let file = parent.path().join("data-file");
        fs::write(&file, b"not a directory").unwrap();
        let file_error = DataDirectory::open_trusted(&file).unwrap_err();
        assert!(file_error.to_string().contains("data-file"));
        assert!(file_error.source().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn workspace_data_symlink_is_rejected_without_external_writes() {
        use std::os::unix::fs::symlink;

        let workspace = TestDirectory::new("data-symlink-workspace");
        let outside = TestDirectory::new("data-symlink-outside");
        fs::create_dir(workspace.path().join(".vsh-runtime")).unwrap();
        symlink(outside.path(), workspace.path().join(".vsh-runtime/data")).unwrap();

        let error = DataDirectory::open_workspace(workspace.path()).unwrap_err();

        assert_eq!(
            error
                .source()
                .unwrap()
                .downcast_ref::<io::Error>()
                .unwrap()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}
