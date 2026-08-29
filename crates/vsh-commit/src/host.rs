use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(unix)]
use cap_std::fs::PermissionsExt;
use cap_std::fs::{Dir, DirEntry, File, Metadata, MetadataExt, OpenOptions, Permissions};
use vsh_store::BlobStore;
use vsh_types::{
    BlobId, ContentVersion, DirectoryDigest, FileStamp, NodeKind, NodeState, PlatformFileId, VPath,
};
use vsh_vfs::{BaseSnapshot, CapturedContent, ContentLoadError, SnapshotBuilder, SnapshotError};

pub(crate) const RUNTIME_DIRECTORY: &str = ".vsh-runtime";
pub(crate) const TRANSACTIONS_DIRECTORY: &str = "transactions";

/// Bounds for eager host metadata traversal; file and link bytes remain lazy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotLimits {
    /// Maximum nodes including the virtual root.
    pub max_nodes: usize,
    /// Maximum directory nesting below the root.
    pub max_depth: usize,
    /// Maximum sum of file and symlink byte sizes represented by metadata.
    pub max_total_file_bytes: u64,
}

impl Default for SnapshotLimits {
    fn default() -> Self {
        Self {
            max_nodes: 250_000,
            max_depth: 128,
            max_total_file_bytes: 16 * 1024 * 1024 * 1024,
        }
    }
}

/// Capability-scoped host filesystem observation failure.
#[derive(Debug)]
pub enum HostError {
    /// A user-visible workspace operation failed.
    Io {
        /// Stable operation label.
        operation: &'static str,
        /// Exact virtual path.
        path: VPath,
        /// Underlying host error.
        source: io::Error,
    },
    /// An internal runtime-directory operation failed.
    InternalIo {
        /// Stable operation label.
        operation: &'static str,
        /// Capability-relative internal path.
        path: PathBuf,
        /// Underlying host error.
        source: io::Error,
    },
    /// The host entry is neither a regular file, directory, nor symbolic link.
    UnsupportedNode {
        /// Rejected path.
        path: VPath,
    },
    /// A host name cannot be represented by portable [`VPath`] UTF-8.
    NonUtf8Name {
        /// Directory containing the name.
        parent: VPath,
        /// Rejected host name.
        name: OsString,
    },
    /// A Windows symbolic-link target cannot be represented portably.
    NonUtf8Symlink {
        /// Rejected link path.
        path: VPath,
    },
    /// The platform did not expose a stable node identity.
    MissingFileIdentity {
        /// Affected path.
        path: VPath,
    },
    /// Metadata changed around a supposedly stable read or enumeration.
    Unstable {
        /// Affected path.
        path: VPath,
        /// First metadata observation.
        before: Box<FileStamp>,
        /// Second metadata observation.
        after: Box<FileStamp>,
    },
    /// Snapshot traversal exceeded a configured bound.
    SnapshotLimit {
        /// Stable limit name.
        limit: &'static str,
        /// Observed value.
        observed: u64,
        /// Configured maximum.
        maximum: u64,
    },
    /// Immutable snapshot construction failed.
    Snapshot(SnapshotError),
}

impl HostError {
    pub(crate) fn io(operation: &'static str, path: &VPath, source: io::Error) -> Self {
        Self::Io {
            operation,
            path: path.clone(),
            source,
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} at {path}: {source}"),
            Self::InternalIo {
                operation,
                path,
                source,
            } => write!(formatter, "{operation} at {}: {source}", path.display()),
            Self::UnsupportedNode { path } => {
                write!(formatter, "unsupported host node at {path}")
            }
            Self::NonUtf8Name { parent, name } => {
                write!(
                    formatter,
                    "non-UTF-8 entry {} below {parent}",
                    name.display()
                )
            }
            Self::NonUtf8Symlink { path } => {
                write!(formatter, "symlink target at {path} is not portable UTF-8")
            }
            Self::MissingFileIdentity { path } => {
                write!(
                    formatter,
                    "host did not expose a stable file identity for {path}"
                )
            }
            Self::Unstable {
                path,
                before,
                after,
            } => write!(
                formatter,
                "host node changed during capture at {path}: {before:?} -> {after:?}"
            ),
            Self::SnapshotLimit {
                limit,
                observed,
                maximum,
            } => write!(
                formatter,
                "snapshot {limit} limit exceeded: observed {observed}, maximum {maximum}"
            ),
            Self::Snapshot(source) => fmt::Display::fmt(source, formatter),
        }
    }
}

impl Error for HostError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } | Self::InternalIo { source, .. } => Some(source),
            Self::Snapshot(source) => Some(source),
            Self::UnsupportedNode { .. }
            | Self::NonUtf8Name { .. }
            | Self::NonUtf8Symlink { .. }
            | Self::MissingFileIdentity { .. }
            | Self::Unstable { .. }
            | Self::SnapshotLimit { .. } => None,
        }
    }
}

pub(crate) fn relative_path(path: &VPath) -> &Path {
    Path::new(path.as_str())
}

#[cfg(not(windows))]
pub(crate) fn sync_dir(dir: &Dir) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.read(true);
    dir.open_with(".", &options)?.into_std().sync_all()
}

#[cfg(windows)]
pub(crate) fn sync_dir(_dir: &Dir) -> io::Result<()> {
    Ok(())
}

pub(crate) fn stamp_at(root: &Dir, path: &VPath) -> Result<Option<FileStamp>, HostError> {
    match root.symlink_metadata(relative_path(path)) {
        Ok(metadata) => stamp_from_metadata(path, &metadata).map(Some),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(HostError::io("read metadata", path, source)),
    }
}

pub(crate) fn stamp_file(file: &File, path: &VPath) -> Result<FileStamp, HostError> {
    let metadata = file
        .metadata()
        .map_err(|source| HostError::io("read open-file metadata", path, source))?;
    stamp_from_metadata(path, &metadata)
}

pub(crate) fn stamp_dir(dir: &Dir, path: &VPath) -> Result<FileStamp, HostError> {
    let metadata = dir
        .dir_metadata()
        .map_err(|source| HostError::io("read open-directory metadata", path, source))?;
    stamp_from_metadata(path, &metadata)
}

#[cfg(unix)]
fn snapshot_entry_stamp(
    _root: &Dir,
    entry: &DirEntry,
    path: &VPath,
) -> Result<FileStamp, HostError> {
    let metadata = entry
        .metadata()
        .map_err(|source| HostError::io("capture node metadata", path, source))?;
    stamp_from_metadata(path, &metadata)
}

#[cfg(windows)]
fn snapshot_entry_stamp(
    root: &Dir,
    _entry: &DirEntry,
    path: &VPath,
) -> Result<FileStamp, HostError> {
    stamp_at(root, path)?.ok_or_else(|| {
        HostError::io(
            "capture node metadata",
            path,
            io::Error::new(io::ErrorKind::NotFound, "node disappeared"),
        )
    })
}

#[cfg(unix)]
fn stamp_from_metadata(path: &VPath, metadata: &Metadata) -> Result<FileStamp, HostError> {
    let kind = node_kind(path, metadata)?;
    Ok(FileStamp {
        kind,
        size: if kind == NodeKind::Directory {
            0
        } else {
            metadata.len()
        },
        mode: MetadataExt::mode(metadata) & 0o7777,
        mtime_ns: i128::from(MetadataExt::mtime(metadata)) * 1_000_000_000
            + i128::from(MetadataExt::mtime_nsec(metadata)),
        ctime_ns: Some(
            i128::from(MetadataExt::ctime(metadata)) * 1_000_000_000
                + i128::from(MetadataExt::ctime_nsec(metadata)),
        ),
        file_id: PlatformFileId {
            high: MetadataExt::dev(metadata),
            low: MetadataExt::ino(metadata),
        },
    })
}

#[cfg(windows)]
fn stamp_from_metadata(path: &VPath, metadata: &Metadata) -> Result<FileStamp, HostError> {
    let kind = node_kind(path, metadata)?;
    let high = MetadataExt::volume_serial_number(metadata)
        .map(u64::from)
        .ok_or_else(|| HostError::MissingFileIdentity { path: path.clone() })?;
    let low = MetadataExt::file_index(metadata)
        .ok_or_else(|| HostError::MissingFileIdentity { path: path.clone() })?;
    let readonly = metadata.permissions().readonly();
    let mode = match (kind, readonly) {
        (NodeKind::Directory, false) => 0o777,
        (NodeKind::Directory, true) => 0o555,
        (NodeKind::File, false) => 0o666,
        (NodeKind::File, true) => 0o444,
        (NodeKind::Symlink, _) => 0o777,
    };
    Ok(FileStamp {
        kind,
        size: if kind == NodeKind::Directory {
            0
        } else {
            MetadataExt::file_size(metadata)
        },
        mode,
        mtime_ns: i128::from(MetadataExt::last_write_time(metadata)) * 100,
        ctime_ns: None,
        file_id: PlatformFileId { high, low },
    })
}

#[cfg(not(any(unix, windows)))]
compile_error!("vsh-commit currently supports Unix and Windows hosts");

fn node_kind(path: &VPath, metadata: &Metadata) -> Result<NodeKind, HostError> {
    let file_type = metadata.file_type();
    if file_type.is_file() {
        Ok(NodeKind::File)
    } else if file_type.is_dir() {
        Ok(NodeKind::Directory)
    } else if file_type.is_symlink() {
        Ok(NodeKind::Symlink)
    } else {
        Err(HostError::UnsupportedNode { path: path.clone() })
    }
}

pub(crate) fn stable_content(root: &Dir, path: &VPath) -> Result<CapturedContent, HostError> {
    let before = stamp_at(root, path)?.ok_or_else(|| {
        HostError::io(
            "capture content",
            path,
            io::Error::new(io::ErrorKind::NotFound, "node disappeared"),
        )
    })?;
    let bytes = match before.kind {
        NodeKind::File => {
            let mut file = root
                .open(relative_path(path))
                .map_err(|source| HostError::io("open file content", path, source))?;
            let opened_before = stamp_file(&file, path)?;
            if opened_before != before {
                return Err(HostError::Unstable {
                    path: path.clone(),
                    before: Box::new(before),
                    after: Box::new(opened_before),
                });
            }
            let mut bytes = Vec::new();
            Read::by_ref(&mut file)
                .take(before.size.saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|source| HostError::io("read file content", path, source))?;
            let opened_after = stamp_file(&file, path)?;
            if opened_after != before {
                return Err(HostError::Unstable {
                    path: path.clone(),
                    before: Box::new(before),
                    after: Box::new(opened_after),
                });
            }
            bytes
        }
        NodeKind::Symlink => {
            let target = root
                .read_link_contents(relative_path(path))
                .map_err(|source| HostError::io("read symlink target", path, source))?;
            symlink_target_bytes(path, &target)?
        }
        NodeKind::Directory => {
            return Err(HostError::io(
                "capture directory content",
                path,
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "directories have no byte content",
                ),
            ));
        }
    };
    let after = stamp_at(root, path)?.ok_or_else(|| {
        HostError::io(
            "capture content",
            path,
            io::Error::new(io::ErrorKind::NotFound, "node disappeared"),
        )
    })?;
    if after != before {
        return Err(HostError::Unstable {
            path: path.clone(),
            before: Box::new(before),
            after: Box::new(after),
        });
    }
    if u64::try_from(bytes.len()).ok() != Some(before.size) {
        return Err(HostError::io(
            "capture stable content",
            path,
            io::Error::new(
                io::ErrorKind::InvalidData,
                "captured byte length does not match metadata",
            ),
        ));
    }
    Ok(CapturedContent {
        bytes,
        before,
        after,
    })
}

#[cfg(unix)]
#[allow(clippy::unnecessary_wraps)]
fn symlink_target_bytes(_path: &VPath, target: &Path) -> Result<Vec<u8>, HostError> {
    use std::os::unix::ffi::OsStrExt;
    Ok(target.as_os_str().as_bytes().to_vec())
}

#[cfg(windows)]
fn symlink_target_bytes(path: &VPath, target: &Path) -> Result<Vec<u8>, HostError> {
    target
        .to_str()
        .map(|value| value.as_bytes().to_vec())
        .ok_or_else(|| HostError::NonUtf8Symlink { path: path.clone() })
}

pub(crate) fn state_matches(
    root: &Dir,
    path: &VPath,
    expected: Option<NodeState>,
) -> Result<(bool, Option<NodeState>), HostError> {
    let Some(stamp) = stamp_at(root, path)? else {
        return Ok((expected.is_none(), None));
    };
    let Some(expected) = expected else {
        return Ok((false, Some(NodeState::from_stamp(stamp))));
    };
    let actual = match expected.content() {
        Some(ContentVersion::Blob(_)) => {
            if stamp.kind != expected.kind()
                || stamp.size != expected.size()
                || stamp.mode != expected.mode()
            {
                NodeState::from_stamp(stamp)
            } else {
                let capture = stable_content(root, path)?;
                let actual_blob = BlobId::digest(&capture.bytes);
                match stamp.kind {
                    NodeKind::File => NodeState::file(actual_blob, stamp.size, stamp.mode),
                    NodeKind::Symlink => NodeState::symlink(actual_blob, stamp.size, stamp.mode),
                    NodeKind::Directory => NodeState::from_stamp(stamp),
                }
            }
        }
        Some(_) => NodeState::from_stamp(stamp),
        None => match stamp.kind {
            NodeKind::Directory => NodeState::directory(stamp.mode),
            NodeKind::File | NodeKind::Symlink => NodeState::from_stamp(stamp),
        },
    };
    let matches = match expected.content() {
        Some(ContentVersion::Stamp(expected_stamp)) => expected_stamp == stamp,
        Some(ContentVersion::Blob(_)) | None => expected == actual,
        Some(_) => false,
    };
    Ok((matches, Some(actual)))
}

pub(crate) fn relocated_state_matches(
    root: &Dir,
    path: &VPath,
    expected: NodeState,
) -> Result<bool, HostError> {
    let Some(actual_stamp) = stamp_at(root, path)? else {
        return Ok(false);
    };
    match expected.content() {
        Some(ContentVersion::Stamp(expected_stamp)) => Ok(expected_stamp.kind == actual_stamp.kind
            && expected_stamp.size == actual_stamp.size
            && expected_stamp.mode == actual_stamp.mode
            && expected_stamp.mtime_ns == actual_stamp.mtime_ns
            && expected_stamp.file_id == actual_stamp.file_id),
        Some(ContentVersion::Blob(_)) | None => {
            state_matches(root, path, Some(expected)).map(|(matches, _)| matches)
        }
        Some(_) => Ok(false),
    }
}

pub(crate) fn content_digest(root: &Dir, path: &VPath) -> Result<BlobId, HostError> {
    stable_content(root, path).map(|capture| BlobId::digest(&capture.bytes))
}

pub(crate) fn directory_digest(
    root: &Dir,
    path: &VPath,
    maximum_entries: usize,
) -> Result<DirectoryDigest, HostError> {
    let before = stamp_at(root, path)?.ok_or_else(|| {
        HostError::io(
            "read directory",
            path,
            io::Error::new(io::ErrorKind::NotFound, "directory disappeared"),
        )
    })?;
    if before.kind != NodeKind::Directory {
        return Err(HostError::io(
            "read directory",
            path,
            io::Error::new(io::ErrorKind::NotADirectory, "node is not a directory"),
        ));
    }
    let mut entries = Vec::new();
    let iterator = root
        .read_dir(relative_path(path))
        .map_err(|source| HostError::io("enumerate directory", path, source))?;
    for entry in iterator {
        let entry = entry.map_err(|source| HostError::io("enumerate directory", path, source))?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| HostError::NonUtf8Name {
            parent: path.clone(),
            name: name.clone(),
        })?;
        if path.is_root() && name == RUNTIME_DIRECTORY {
            continue;
        }
        if entries.len() >= maximum_entries {
            return Err(HostError::SnapshotLimit {
                limit: "directory-entries",
                observed: u64::try_from(entries.len())
                    .unwrap_or(u64::MAX)
                    .saturating_add(1),
                maximum: u64::try_from(maximum_entries).unwrap_or(u64::MAX),
            });
        }
        let child = path.join(name).map_err(|source| {
            HostError::io("normalize directory entry", path, io::Error::other(source))
        })?;
        let stamp = stamp_at(root, &child)?.ok_or_else(|| {
            HostError::io(
                "read directory entry metadata",
                &child,
                io::Error::new(io::ErrorKind::NotFound, "entry disappeared"),
            )
        })?;
        entries.push((child, NodeState::from_stamp(stamp)));
    }
    entries.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let after = stamp_at(root, path)?.ok_or_else(|| {
        HostError::io(
            "read directory",
            path,
            io::Error::new(io::ErrorKind::NotFound, "directory disappeared"),
        )
    })?;
    if before != after {
        return Err(HostError::Unstable {
            path: path.clone(),
            before: Box::new(before),
            after: Box::new(after),
        });
    }
    Ok(DirectoryDigest::digest_entries(
        entries.iter().map(|(child, state)| (child, *state)),
    ))
}

#[allow(clippy::too_many_lines)]
pub(crate) fn capture_snapshot(
    root: &Arc<Dir>,
    store: BlobStore,
    limits: SnapshotLimits,
) -> Result<BaseSnapshot, HostError> {
    let root_path = VPath::root();
    let root_stamp = stamp_dir(root, &root_path)?;
    let mut builder = SnapshotBuilder::with_root_stamp(store, root_stamp);
    let mut pending = vec![(root_path, 0_usize)];
    let mut node_count = 1_usize;
    let mut total_bytes = 0_u64;

    while let Some((parent, depth)) = pending.pop() {
        let before = stamp_at(root, &parent)?.ok_or_else(|| {
            HostError::io(
                "capture directory",
                &parent,
                io::Error::new(io::ErrorKind::NotFound, "directory disappeared"),
            )
        })?;
        let iterator = root
            .read_dir(relative_path(&parent))
            .map_err(|source| HostError::io("enumerate snapshot directory", &parent, source))?;
        let mut children = Vec::new();
        for entry in iterator {
            let entry = entry
                .map_err(|source| HostError::io("enumerate snapshot directory", &parent, source))?;
            let raw_name = entry.file_name();
            let name = raw_name.to_str().ok_or_else(|| HostError::NonUtf8Name {
                parent: parent.clone(),
                name: raw_name.clone(),
            })?;
            if parent.is_root() && name == RUNTIME_DIRECTORY {
                continue;
            }
            let child = parent.join(name).map_err(|source| {
                HostError::io("normalize snapshot path", &parent, io::Error::other(source))
            })?;
            let stamp = snapshot_entry_stamp(root, &entry, &child)?;
            children.push((child, stamp));
        }
        children.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        let after = stamp_at(root, &parent)?.ok_or_else(|| {
            HostError::io(
                "capture directory",
                &parent,
                io::Error::new(io::ErrorKind::NotFound, "directory disappeared"),
            )
        })?;
        if before != after {
            return Err(HostError::Unstable {
                path: parent,
                before: Box::new(before),
                after: Box::new(after),
            });
        }

        for (child, stamp) in children {
            node_count = node_count.saturating_add(1);
            if node_count > limits.max_nodes {
                return Err(HostError::SnapshotLimit {
                    limit: "node-count",
                    observed: node_count as u64,
                    maximum: limits.max_nodes as u64,
                });
            }
            match stamp.kind {
                NodeKind::Directory => {
                    let next_depth = depth.saturating_add(1);
                    if next_depth > limits.max_depth {
                        return Err(HostError::SnapshotLimit {
                            limit: "depth",
                            observed: next_depth as u64,
                            maximum: limits.max_depth as u64,
                        });
                    }
                    builder
                        .add_stamped_directory(child.clone(), stamp)
                        .map_err(HostError::Snapshot)?;
                    pending.push((child, next_depth));
                }
                NodeKind::File | NodeKind::Symlink => {
                    total_bytes = total_bytes.saturating_add(stamp.size);
                    if total_bytes > limits.max_total_file_bytes {
                        return Err(HostError::SnapshotLimit {
                            limit: "total-file-bytes",
                            observed: total_bytes,
                            maximum: limits.max_total_file_bytes,
                        });
                    }
                    let loader_root = Arc::clone(root);
                    let loader_path = child.clone();
                    builder
                        .add_lazy(child, stamp, move |expected| {
                            let captured = stable_content(&loader_root, &loader_path)
                                .map_err(|source| ContentLoadError::new(source.to_string()))?;
                            if captured.before != expected || captured.after != expected {
                                return Err(ContentLoadError::new(
                                    "snapshot node changed before lazy capture",
                                ));
                            }
                            Ok(captured)
                        })
                        .map_err(HostError::Snapshot)?;
                }
            }
        }
    }
    builder.build().map_err(HostError::Snapshot)
}

pub(crate) fn open_or_create_real_dir(parent: &Dir, name: &str) -> io::Result<Dir> {
    match parent.create_dir(name) {
        Ok(()) => {}
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {}
        Err(source) => return Err(source),
    }
    open_real_dir(parent, name)
}

pub(crate) fn open_real_dir(parent: &Dir, name: &str) -> io::Result<Dir> {
    let before = parent.symlink_metadata(name)?;
    if !before.is_dir() || before.is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "internal VSH path is not a real directory",
        ));
    }
    let directory = parent.open_dir(name)?;
    let opened = directory.dir_metadata()?;
    let after = parent.symlink_metadata(name)?;
    if !after.is_dir() || after.is_symlink() || !metadata_identity_matches(&opened, &after) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "internal VSH directory changed while it was being pinned",
        ));
    }
    Ok(directory)
}

pub(crate) fn open_real_file(parent: &Dir, name: &str) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    open_real_file_with(parent, name, &options)
}

fn open_real_file_with(parent: &Dir, name: &str, options: &OpenOptions) -> io::Result<File> {
    let before = parent.symlink_metadata(name)?;
    if !before.is_file() || before.is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "internal VSH path is not a real file",
        ));
    }
    let file = parent.open_with(name, options)?;
    let opened = file.metadata()?;
    let after = parent.symlink_metadata(name)?;
    if !after.is_file() || after.is_symlink() || !metadata_identity_matches(&opened, &after) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "internal VSH file changed while it was being pinned",
        ));
    }
    Ok(file)
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

pub(crate) fn create_new_file(dir: &Dir, name: &str) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    dir.open_with(name, &options)
}

pub(crate) fn open_coordination_file(
    dir: &Dir,
    name: &'static str,
) -> Result<std::fs::File, HostError> {
    let path = VPath::parse(name).expect("internal coordination filename is a valid VPath");
    let mut create = OpenOptions::new();
    create.read(true).write(true).create_new(true);
    let file = match dir.open_with(name, &create) {
        Ok(file) => file,
        Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
            let mut existing = OpenOptions::new();
            existing.read(true).write(true);
            open_real_file_with(dir, name, &existing).map_err(|source| HostError::InternalIo {
                operation: "open workspace coordination file",
                path: PathBuf::from(name),
                source,
            })?
        }
        Err(source) => {
            return Err(HostError::InternalIo {
                operation: "create workspace coordination file",
                path: PathBuf::from(name),
                source,
            });
        }
    };
    let opened = stamp_file(&file, &path)?;
    let named = stamp_at(dir, &path)?.ok_or_else(|| HostError::Unstable {
        path: path.clone(),
        before: Box::new(opened),
        after: Box::new(opened),
    })?;
    if opened.kind != NodeKind::File
        || opened.file_id != named.file_id
        || named.kind != NodeKind::File
    {
        return Err(HostError::Unstable {
            path,
            before: Box::new(named),
            after: Box::new(opened),
        });
    }
    file.sync_all().map_err(|source| HostError::InternalIo {
        operation: "sync workspace coordination file",
        path: PathBuf::from(name),
        source,
    })?;
    Ok(file.into_std())
}

pub(crate) fn set_file_mode(file: &File, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        file.set_permissions(Permissions::from_mode(mode))
    }
    #[cfg(windows)]
    {
        let mut permissions = file.metadata()?.permissions();
        permissions.set_readonly(mode & 0o200 == 0);
        file.set_permissions(permissions)
    }
}

pub(crate) fn set_dir_mode(dir: &Dir, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        dir.set_permissions(".", Permissions::from_mode(mode))
    }
    #[cfg(windows)]
    {
        let file = dir.try_clone()?.into_std_file();
        let mut permissions = file.metadata()?.permissions();
        permissions.set_readonly(mode & 0o200 == 0);
        file.set_permissions(permissions)
    }
}

pub(crate) fn witness_matches(
    root: &Dir,
    path: &VPath,
    kind: NodeKind,
    file_id: PlatformFileId,
) -> Result<bool, HostError> {
    Ok(stamp_at(root, path)?.is_some_and(|stamp| stamp.kind == kind && stamp.file_id == file_id))
}

pub(crate) fn validate_symlink_target(path: &VPath, bytes: &[u8]) -> Result<PathBuf, HostError> {
    let target =
        std::str::from_utf8(bytes).map_err(|_| HostError::NonUtf8Symlink { path: path.clone() })?;
    if target.is_empty() {
        return Err(HostError::io(
            "validate symlink target",
            path,
            io::Error::new(io::ErrorKind::InvalidInput, "symlink target is empty"),
        ));
    }
    let portable = target.replace('\\', "/");
    let parent = path.parent().unwrap_or_else(VPath::root);
    parent.join(&portable).map_err(|source| {
        HostError::io(
            "validate symlink target",
            path,
            io::Error::new(io::ErrorKind::PermissionDenied, source),
        )
    })?;
    Ok(PathBuf::from(portable))
}

pub(crate) fn create_staged_symlink(
    stage: &Dir,
    name: &str,
    root: &Dir,
    path: &VPath,
    target: &Path,
) -> Result<(), HostError> {
    #[cfg(unix)]
    {
        let _ = root;
        stage
            .symlink_contents(target, name)
            .map_err(|source| HostError::io("create symbolic link", path, source))
    }
    #[cfg(windows)]
    {
        let parent = path.parent().unwrap_or_else(VPath::root);
        let resolved = parent.join(&target.to_string_lossy()).map_err(|source| {
            HostError::io(
                "resolve symbolic-link type",
                path,
                io::Error::new(io::ErrorKind::InvalidInput, source),
            )
        })?;
        let target_is_dir = root
            .symlink_metadata(relative_path(&resolved))
            .is_ok_and(|metadata| metadata.is_dir());
        let result = if target_is_dir {
            stage.symlink_dir(target, name)
        } else {
            stage.symlink_file(target, name)
        };
        result.map_err(|source| HostError::io("create symbolic link", path, source))
    }
}
