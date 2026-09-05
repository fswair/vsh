//! Immutable snapshots and the copy-on-write virtual filesystem used by VSH.
//!
//! This crate has no host commit capability. Every mutation lands in an in-memory
//! overlay, and the only durable writes it can perform are immutable blob-store puts.

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use std::error::Error;
use std::fmt;
use std::ops::Bound::{Included, Unbounded};
use std::sync::{Arc, Mutex, OnceLock};

use vsh_store::{BlobStore, BlobStoreError};
use vsh_types::{
    BlobId, DiffDigest, DiffEntry, DiffKind, DirectoryDigest, FileStamp, NodeKind, NodeState,
    SnapshotId, VPath, VPathError,
};

#[cfg(not(windows))]
const DEFAULT_FILE_MODE: u32 = 0o644;
#[cfg(windows)]
const DEFAULT_FILE_MODE: u32 = 0o666;

#[cfg(not(windows))]
const fn platform_directory_mode(mode: u32) -> u32 {
    mode
}

#[cfg(windows)]
const fn platform_directory_mode(mode: u32) -> u32 {
    if mode & 0o200 == 0 { 0o555 } else { 0o777 }
}

/// Bytes captured between two metadata observations of the same host node.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapturedContent {
    /// Captured bytes.
    pub bytes: Vec<u8>,
    /// Metadata immediately before the read.
    pub before: FileStamp,
    /// Metadata immediately after the read.
    pub after: FileStamp,
}

/// Error returned by a lazy snapshot content loader.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentLoadError {
    message: String,
}

impl ContentLoadError {
    /// Construct an adapter-neutral lazy-load failure.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for ContentLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ContentLoadError {}

/// Capability-scoped provider for one lazily captured snapshot node.
///
/// Implementations must open relative to their already-authorized root and must not
/// follow a different node in place of the expected one. VSH independently verifies
/// the returned before/after stamps and byte length.
pub trait ContentLoader: Send + Sync {
    /// Capture stable content for `expected`.
    ///
    /// # Errors
    ///
    /// Returns [`ContentLoadError`] when the host read cannot be completed safely.
    fn load(&self, expected: FileStamp) -> Result<CapturedContent, ContentLoadError>;
}

impl<F> ContentLoader for F
where
    F: Fn(FileStamp) -> Result<CapturedContent, ContentLoadError> + Send + Sync,
{
    fn load(&self, expected: FileStamp) -> Result<CapturedContent, ContentLoadError> {
        self(expected)
    }
}

struct LazyContent {
    expected: FileStamp,
    loader: Arc<dyn ContentLoader>,
    captured: OnceLock<BlobId>,
    load_lock: Mutex<()>,
}

#[derive(Clone)]
enum ContentHandle {
    None,
    Materialized(BlobId),
    Lazy(Arc<LazyContent>),
}

#[derive(Clone)]
struct SnapshotNode {
    state: NodeState,
    content: ContentHandle,
}

impl SnapshotNode {
    fn directory(mode: u32) -> Self {
        Self {
            state: NodeState::directory(mode),
            content: ContentHandle::None,
        }
    }

    fn stamped_directory(stamp: FileStamp) -> Self {
        debug_assert_eq!(stamp.kind, NodeKind::Directory);
        Self {
            state: NodeState::from_stamp(stamp),
            content: ContentHandle::None,
        }
    }

    fn materialized(kind: NodeKind, id: BlobId, size: u64, mode: u32) -> Self {
        let state = match kind {
            NodeKind::File => NodeState::file(id, size, mode),
            NodeKind::Symlink => NodeState::symlink(id, size, mode),
            NodeKind::Directory => NodeState::directory(mode),
        };
        let content = match kind {
            NodeKind::Directory => ContentHandle::None,
            NodeKind::File | NodeKind::Symlink => ContentHandle::Materialized(id),
        };
        Self { state, content }
    }

    fn lazy(stamp: FileStamp, loader: Arc<dyn ContentLoader>) -> Self {
        Self {
            state: NodeState::from_stamp(stamp),
            content: ContentHandle::Lazy(Arc::new(LazyContent {
                expected: stamp,
                loader,
                captured: OnceLock::new(),
                load_lock: Mutex::new(()),
            })),
        }
    }

    fn state(&self) -> NodeState {
        match &self.content {
            ContentHandle::Lazy(lazy) => lazy
                .captured
                .get()
                .copied()
                .and_then(|blob| self.state.with_blob(blob, self.state.size()))
                .unwrap_or(self.state),
            ContentHandle::None | ContentHandle::Materialized(_) => self.state,
        }
    }

    fn expected_state(&self) -> NodeState {
        self.state
    }

    fn read(&self, path: &VPath, store: &BlobStore) -> Result<(BlobId, Vec<u8>), SnapshotError> {
        match &self.content {
            ContentHandle::None => Err(SnapshotError::ContentUnavailable {
                path: path.clone(),
                kind: self.state.kind(),
            }),
            ContentHandle::Materialized(id) => {
                let bytes = store.get(*id).map_err(SnapshotError::Store)?;
                Ok((*id, bytes))
            }
            ContentHandle::Lazy(lazy) => {
                if let Some(id) = lazy.captured.get().copied() {
                    let bytes = store.get(id).map_err(SnapshotError::Store)?;
                    return Ok((id, bytes));
                }
                let _load_guard = lazy
                    .load_lock
                    .lock()
                    .map_err(|_| SnapshotError::LazyStatePoisoned { path: path.clone() })?;
                if let Some(id) = lazy.captured.get().copied() {
                    let bytes = store.get(id).map_err(SnapshotError::Store)?;
                    return Ok((id, bytes));
                }

                let captured = lazy.loader.load(lazy.expected).map_err(|source| {
                    SnapshotError::ContentLoad {
                        path: path.clone(),
                        source,
                    }
                })?;
                if captured.before != lazy.expected || captured.after != lazy.expected {
                    return Err(SnapshotError::StaleContent {
                        path: path.clone(),
                        expected: Box::new(lazy.expected),
                        before: Box::new(captured.before),
                        after: Box::new(captured.after),
                    });
                }
                if u64::try_from(captured.bytes.len()).ok() != Some(lazy.expected.size) {
                    return Err(SnapshotError::ContentSizeMismatch {
                        path: path.clone(),
                        expected: lazy.expected.size,
                        actual: captured.bytes.len(),
                    });
                }
                let id = store.put(&captured.bytes).map_err(SnapshotError::Store)?;
                lazy.captured
                    .set(id)
                    .map_err(|_| SnapshotError::LazyStatePoisoned { path: path.clone() })?;
                Ok((id, captured.bytes))
            }
        }
    }

    fn materialized_state(
        &self,
        path: &VPath,
        store: &BlobStore,
    ) -> Result<NodeState, SnapshotError> {
        if self.state.kind() == NodeKind::Directory {
            return Ok(self.state);
        }
        let (blob, bytes) = self.read(path, store)?;
        Ok(self
            .state
            .with_blob(blob, bytes.len() as u64)
            .expect("non-directory nodes accept blob content"))
    }

    fn is_lazy(&self) -> bool {
        matches!(self.content, ContentHandle::Lazy(_))
    }

    fn is_materialized(&self) -> bool {
        match &self.content {
            ContentHandle::Materialized(_) => true,
            ContentHandle::Lazy(lazy) => lazy.captured.get().is_some(),
            ContentHandle::None => false,
        }
    }
}

/// Builder for one immutable snapshot manifest.
pub struct SnapshotBuilder {
    store: BlobStore,
    nodes: BTreeMap<VPath, Arc<SnapshotNode>>,
}

impl SnapshotBuilder {
    /// Start a snapshot containing only the virtual root directory.
    #[must_use]
    pub fn new(store: BlobStore) -> Self {
        let mut nodes = BTreeMap::new();
        nodes.insert(VPath::root(), Arc::new(SnapshotNode::directory(0o755)));
        Self { store, nodes }
    }

    /// Start a host snapshot whose root identity was captured without following links.
    ///
    /// # Panics
    ///
    /// Panics if `root_stamp` does not describe a directory. Host adapters construct
    /// this value directly from an already-open directory capability.
    #[must_use]
    pub fn with_root_stamp(store: BlobStore, root_stamp: FileStamp) -> Self {
        assert_eq!(root_stamp.kind, NodeKind::Directory);
        let mut nodes = BTreeMap::new();
        nodes.insert(
            VPath::root(),
            Arc::new(SnapshotNode::stamped_directory(root_stamp)),
        );
        Self { store, nodes }
    }

    /// Add a directory to the immutable manifest.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError::DuplicatePath`] when `path` already exists.
    pub fn add_directory(&mut self, path: VPath, mode: u32) -> Result<&mut Self, SnapshotError> {
        self.insert(path, SnapshotNode::directory(mode))?;
        Ok(self)
    }

    /// Add a host directory while retaining its race-detection identity.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate paths or a non-directory stamp.
    pub fn add_stamped_directory(
        &mut self,
        path: VPath,
        stamp: FileStamp,
    ) -> Result<&mut Self, SnapshotError> {
        if stamp.kind != NodeKind::Directory {
            return Err(SnapshotError::ExpectedDirectoryStamp { path, stamp });
        }
        self.insert(path, SnapshotNode::stamped_directory(stamp))?;
        Ok(self)
    }

    /// Add and content-address a regular file.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate paths or blob-store failures.
    pub fn add_file(
        &mut self,
        path: VPath,
        bytes: &[u8],
        mode: u32,
    ) -> Result<&mut Self, SnapshotError> {
        let blob = self.store.put(bytes).map_err(SnapshotError::Store)?;
        self.insert(
            path,
            SnapshotNode::materialized(NodeKind::File, blob, bytes.len() as u64, mode),
        )?;
        Ok(self)
    }

    /// Add an opaque symbolic link without following its target.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate paths or blob-store failures.
    pub fn add_symlink(
        &mut self,
        path: VPath,
        target: &[u8],
        mode: u32,
    ) -> Result<&mut Self, SnapshotError> {
        let blob = self.store.put(target).map_err(SnapshotError::Store)?;
        self.insert(
            path,
            SnapshotNode::materialized(NodeKind::Symlink, blob, target.len() as u64, mode),
        )?;
        Ok(self)
    }

    /// Add a file or symlink whose content will be captured on first read.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate paths or a directory stamp.
    pub fn add_lazy<L>(
        &mut self,
        path: VPath,
        stamp: FileStamp,
        loader: L,
    ) -> Result<&mut Self, SnapshotError>
    where
        L: ContentLoader + 'static,
    {
        if stamp.kind == NodeKind::Directory {
            return Err(SnapshotError::LazyDirectory { path });
        }
        self.insert(path, SnapshotNode::lazy(stamp, Arc::new(loader)))?;
        Ok(self)
    }

    /// Validate parent relationships and freeze the snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when a parent is absent or not a directory.
    pub fn build(self) -> Result<BaseSnapshot, SnapshotError> {
        let mut children: BTreeMap<VPath, BTreeSet<VPath>> = BTreeMap::new();
        for path in self.nodes.keys().filter(|path| !path.is_root()) {
            let parent = path.parent().unwrap_or_else(VPath::root);
            let Some(parent_node) = self.nodes.get(&parent) else {
                return Err(SnapshotError::MissingParent {
                    path: path.clone(),
                    parent,
                });
            };
            if parent_node.state.kind() != NodeKind::Directory {
                return Err(SnapshotError::ParentNotDirectory {
                    path: path.clone(),
                    parent,
                });
            }
            children.entry(parent).or_default().insert(path.clone());
        }
        let id = snapshot_id(&self.nodes);
        Ok(BaseSnapshot {
            inner: Arc::new(SnapshotInner {
                id,
                nodes: self.nodes,
                children,
                store: self.store,
            }),
        })
    }

    fn insert(&mut self, path: VPath, node: SnapshotNode) -> Result<(), SnapshotError> {
        match self.nodes.entry(path) {
            Entry::Occupied(entry) => Err(SnapshotError::DuplicatePath {
                path: entry.key().clone(),
            }),
            Entry::Vacant(entry) => {
                entry.insert(Arc::new(node));
                Ok(())
            }
        }
    }
}

fn snapshot_id(nodes: &BTreeMap<VPath, Arc<SnapshotNode>>) -> SnapshotId {
    let mut canonical = Vec::new();
    for (path, node) in nodes {
        encode_path(path, &mut canonical);
        node.state.encode_canonical(&mut canonical);
    }
    SnapshotId::digest_manifest(&canonical)
}

struct SnapshotInner {
    id: SnapshotId,
    nodes: BTreeMap<VPath, Arc<SnapshotNode>>,
    children: BTreeMap<VPath, BTreeSet<VPath>>,
    store: BlobStore,
}

/// An immutable metadata manifest with content that becomes immutable on first capture.
#[derive(Clone)]
pub struct BaseSnapshot {
    inner: Arc<SnapshotInner>,
}

impl BaseSnapshot {
    /// Return the stable manifest identity.
    #[must_use]
    pub fn id(&self) -> SnapshotId {
        self.inner.id
    }

    /// Return the number of manifest nodes, including the virtual root.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.nodes.len()
    }

    /// Return whether the manifest contains no user-visible nodes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 1
    }

    /// Return snapshot materialization metrics.
    #[must_use]
    pub fn metrics(&self) -> SnapshotMetrics {
        let lazy_nodes = self
            .inner
            .nodes
            .values()
            .filter(|node| node.is_lazy())
            .count();
        let materialized_content_nodes = self
            .inner
            .nodes
            .values()
            .filter(|node| node.is_materialized())
            .count();
        SnapshotMetrics {
            node_count: self.len(),
            lazy_content_nodes: lazy_nodes,
            materialized_content_nodes,
        }
    }

    fn node(&self, path: &VPath) -> Option<Arc<SnapshotNode>> {
        self.inner.nodes.get(path).cloned()
    }

    fn direct_children(&self, path: &VPath) -> impl Iterator<Item = VPath> + '_ {
        self.inner
            .children
            .get(path)
            .into_iter()
            .flat_map(|children| children.iter().cloned())
    }

    fn subtree_paths(&self, root: &VPath) -> Vec<VPath> {
        if !self.inner.nodes.contains_key(root) {
            return Vec::new();
        }
        let mut paths = Vec::new();
        let mut pending = vec![root.clone()];
        while let Some(path) = pending.pop() {
            if let Some(children) = self.inner.children.get(&path) {
                pending.extend(children.iter().rev().cloned());
            }
            paths.push(path);
        }
        paths.sort_unstable();
        paths
    }

    fn directory_digest(&self, path: &VPath) -> DirectoryDigest {
        let children = self.direct_children(path).collect::<Vec<_>>();
        DirectoryDigest::digest_entries(
            children
                .iter()
                .map(|child| (child, self.inner.nodes[child].expected_state())),
        )
    }

    fn store(&self) -> &BlobStore {
        &self.inner.store
    }
}

/// Snapshot content-capture and manifest validation failure.
#[derive(Debug)]
#[non_exhaustive]
pub enum SnapshotError {
    /// Immutable blob storage failed.
    Store(BlobStoreError),
    /// The manifest contains the same path more than once.
    DuplicatePath {
        /// Duplicate path.
        path: VPath,
    },
    /// A manifest node has no parent directory.
    MissingParent {
        /// Child path.
        path: VPath,
        /// Missing parent.
        parent: VPath,
    },
    /// A manifest node's parent is not a directory.
    ParentNotDirectory {
        /// Child path.
        path: VPath,
        /// Non-directory parent.
        parent: VPath,
    },
    /// Directories have no lazy byte content.
    LazyDirectory {
        /// Rejected path.
        path: VPath,
    },
    /// A host-directory builder method received another node kind.
    ExpectedDirectoryStamp {
        /// Rejected path.
        path: VPath,
        /// Non-directory stamp.
        stamp: FileStamp,
    },
    /// A directory was used as byte content.
    ContentUnavailable {
        /// Requested path.
        path: VPath,
        /// Actual node kind.
        kind: NodeKind,
    },
    /// An authorized content loader failed.
    ContentLoad {
        /// Requested path.
        path: VPath,
        /// Adapter error.
        source: ContentLoadError,
    },
    /// Metadata changed around a lazy content read.
    StaleContent {
        /// Requested path.
        path: VPath,
        /// Snapshot metadata.
        expected: Box<FileStamp>,
        /// Metadata before the read.
        before: Box<FileStamp>,
        /// Metadata after the read.
        after: Box<FileStamp>,
    },
    /// Captured byte count did not match the immutable stamp.
    ContentSizeMismatch {
        /// Requested path.
        path: VPath,
        /// Size from the snapshot stamp.
        expected: u64,
        /// Actual byte count.
        actual: usize,
    },
    /// A previous panic poisoned the lazy capture lock; VSH fails closed.
    LazyStatePoisoned {
        /// Affected path.
        path: VPath,
    },
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(source) => write!(formatter, "blob store failure: {source}"),
            Self::DuplicatePath { path } => write!(formatter, "duplicate snapshot path: {path}"),
            Self::MissingParent { path, parent } => {
                write!(
                    formatter,
                    "snapshot path {path} has missing parent {parent}"
                )
            }
            Self::ParentNotDirectory { path, parent } => {
                write!(
                    formatter,
                    "snapshot path {path} has non-directory parent {parent}"
                )
            }
            Self::LazyDirectory { path } => {
                write!(formatter, "directory {path} cannot have lazy byte content")
            }
            Self::ExpectedDirectoryStamp { path, stamp } => write!(
                formatter,
                "expected a directory stamp for {path}, got {:?}",
                stamp.kind
            ),
            Self::ContentUnavailable { path, kind } => {
                write!(formatter, "{path} has no readable byte content ({kind:?})")
            }
            Self::ContentLoad { path, source } => {
                write!(
                    formatter,
                    "failed to capture snapshot content for {path}: {source}"
                )
            }
            Self::StaleContent { path, .. } => {
                write!(formatter, "snapshot content changed while reading {path}")
            }
            Self::ContentSizeMismatch {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "snapshot content size changed for {path}: expected {expected}, got {actual}"
            ),
            Self::LazyStatePoisoned { path } => {
                write!(formatter, "lazy snapshot state was poisoned for {path}")
            }
        }
    }
}

impl Error for SnapshotError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Store(source) => Some(source),
            Self::ContentLoad { source, .. } => Some(source),
            Self::DuplicatePath { .. }
            | Self::MissingParent { .. }
            | Self::ParentNotDirectory { .. }
            | Self::LazyDirectory { .. }
            | Self::ExpectedDirectoryStamp { .. }
            | Self::ContentUnavailable { .. }
            | Self::StaleContent { .. }
            | Self::ContentSizeMismatch { .. }
            | Self::LazyStatePoisoned { .. } => None,
        }
    }
}

/// Observable snapshot size and lazy-materialization state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotMetrics {
    /// Total manifest nodes including root.
    pub node_count: usize,
    /// Nodes configured for lazy capture.
    pub lazy_content_nodes: usize,
    /// File/link nodes whose bytes are already in the blob store.
    pub materialized_content_nodes: usize,
}

#[derive(Clone)]
struct ResolvedNode {
    node: Arc<SnapshotNode>,
    base_origin: Option<VPath>,
}

#[derive(Clone)]
enum OverlayEntry {
    Present(ResolvedNode),
    Tombstone,
}

/// Dependency observed while virtual code was executing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReadObservation {
    /// Base metadata expected at commit time. `Some(None)` means expected missing.
    pub metadata: Option<Option<NodeState>>,
    /// Exact base content read by the program.
    pub content: Option<BlobId>,
    /// Exact base directory listing read by the program.
    pub directory: Option<DirectoryDigest>,
}

/// Base state that must still hold before a path may be written.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WritePrecondition {
    /// Expected base state, or `None` when the path must remain absent.
    pub expected: Option<NodeState>,
}

/// Source of an observed virtual filesystem effect.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EffectOrigin {
    /// A direct typed virtual filesystem operation.
    VirtualFs,
    /// A typed Monty OS call; populated by the Monty adapter in Phase 3.
    MontyOsCall,
    /// A high-level VSH function called from inside a Monty program.
    MontyToolCall,
}

/// Semantic event emitted by the operation that actually observed or changed state.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Effect {
    /// Metadata or existence was observed.
    MetadataRead {
        /// Observed path.
        path: VPath,
        /// State seen by the program.
        state: Option<NodeState>,
    },
    /// File content was observed.
    ContentRead {
        /// Observed path.
        path: VPath,
        /// Exact content identity.
        blob: BlobId,
    },
    /// Directory entries were observed.
    DirectoryRead {
        /// Observed directory.
        path: VPath,
        /// Digest of the listing seen by the program.
        digest: DirectoryDigest,
    },
    /// A path was created.
    Create {
        /// Created path.
        path: VPath,
        /// New state.
        after: NodeState,
    },
    /// Regular file content was replaced.
    ModifyContent {
        /// Changed path.
        path: VPath,
        /// State before the operation.
        before: NodeState,
        /// State after the operation.
        after: NodeState,
    },
    /// A path was deleted.
    Delete {
        /// Deleted path.
        path: VPath,
        /// State before deletion.
        before: NodeState,
    },
    /// A subtree was moved without host effects.
    Rename {
        /// Source root.
        from: VPath,
        /// Destination root.
        to: VPath,
        /// Source state before the move.
        before: NodeState,
        /// Destination state after the move.
        after: NodeState,
    },
}

/// One sequence-numbered observed effect.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EffectEvent {
    /// Monotonic transaction-local sequence number.
    pub sequence: u64,
    /// Adapter that originated the operation.
    pub origin: EffectOrigin,
    /// Observed semantic event.
    pub effect: Effect,
}

/// Stable, path-ordered virtual filesystem diff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalDiff {
    entries: Vec<DiffEntry>,
    digest: DiffDigest,
    metrics: CanonicalDiffMetrics,
}

impl CanonicalDiff {
    /// Reconstruct a canonical diff from a trusted artifact decoder.
    ///
    /// Entries must be strictly path-ordered, non-root, semantically classified, and
    /// represent an actual state change. The digest is always recomputed.
    ///
    /// # Errors
    ///
    /// Returns [`VfsError::InvalidCanonicalDiff`] when an artifact violates a
    /// canonical invariant.
    pub fn from_entries(entries: Vec<DiffEntry>) -> Result<Self, VfsError> {
        let mut previous: Option<&VPath> = None;
        let mut materialized_after_bytes = 0_u64;
        for entry in &entries {
            if entry.path.is_root() {
                return Err(VfsError::InvalidCanonicalDiff {
                    path: Some(entry.path.clone()),
                    reason: "the workspace root cannot appear in a diff",
                });
            }
            if previous.is_some_and(|path| path >= &entry.path) {
                return Err(VfsError::InvalidCanonicalDiff {
                    path: Some(entry.path.clone()),
                    reason: "diff paths are not strictly ordered",
                });
            }
            if entry.before == entry.after {
                return Err(VfsError::InvalidCanonicalDiff {
                    path: Some(entry.path.clone()),
                    reason: "diff entry does not change state",
                });
            }
            if entry.kind != classify_diff(entry.before, entry.after) {
                return Err(VfsError::InvalidCanonicalDiff {
                    path: Some(entry.path.clone()),
                    reason: "diff kind does not match before and after states",
                });
            }
            materialized_after_bytes =
                materialized_after_bytes.saturating_add(entry.after.map_or(0, NodeState::size));
            previous = Some(&entry.path);
        }
        let mut canonical = Vec::new();
        for entry in &entries {
            encode_diff_entry(entry, &mut canonical);
        }
        Ok(Self {
            metrics: CanonicalDiffMetrics {
                candidate_paths: entries.len(),
                expanded_delete_paths: 0,
                changed_paths: entries.len(),
                materialized_after_bytes,
            },
            digest: DiffDigest::digest_canonical(&canonical),
            entries,
        })
    }

    /// Return canonical entries ordered by normalized path.
    #[must_use]
    pub fn entries(&self) -> &[DiffEntry] {
        &self.entries
    }

    /// Return the domain-separated digest of the canonical encoding.
    #[must_use]
    pub const fn digest(&self) -> DiffDigest {
        self.digest
    }

    /// Return whether final virtual state equals the base snapshot.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Return work-size metrics for performance and budget enforcement.
    #[must_use]
    pub const fn metrics(&self) -> CanonicalDiffMetrics {
        self.metrics
    }
}

/// Work performed while deriving one canonical diff.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalDiffMetrics {
    /// Unique paths compared against base state.
    pub candidate_paths: usize,
    /// Base paths expanded because an ancestor subtree was deleted.
    pub expanded_delete_paths: usize,
    /// Final changed paths emitted in the diff.
    pub changed_paths: usize,
    /// Bytes represented by materialized candidate after-states.
    pub materialized_after_bytes: u64,
}

/// Copy-on-write filesystem over one immutable snapshot.
pub struct VirtualFs {
    base: BaseSnapshot,
    overlay: BTreeMap<VPath, OverlayEntry>,
    effects: Vec<EffectEvent>,
    read_set: BTreeMap<VPath, ReadObservation>,
    write_set: BTreeMap<VPath, WritePrecondition>,
    next_sequence: u64,
    effect_origin: EffectOrigin,
}

impl VirtualFs {
    /// Begin a new isolated virtual transaction.
    #[must_use]
    pub fn new(base: BaseSnapshot) -> Self {
        Self {
            base,
            overlay: BTreeMap::new(),
            effects: Vec::new(),
            read_set: BTreeMap::new(),
            write_set: BTreeMap::new(),
            next_sequence: 0,
            effect_origin: EffectOrigin::VirtualFs,
        }
    }

    /// Return the number of base manifest nodes, including the virtual root.
    #[must_use]
    pub fn base_node_count(&self) -> usize {
        self.base.len()
    }

    /// Run an operation while attributing every emitted effect to `origin`.
    ///
    /// Adapters use this narrow scope to preserve the typed source of observations
    /// without duplicating filesystem semantics. Nested scopes restore the previous
    /// origin when the operation returns.
    pub fn with_effect_origin<T>(
        &mut self,
        origin: EffectOrigin,
        operation: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let previous = self.effect_origin;
        self.effect_origin = origin;
        let result = operation(self);
        self.effect_origin = previous;
        result
    }

    /// Return the immutable base snapshot identity.
    #[must_use]
    pub fn base_snapshot_id(&self) -> SnapshotId {
        self.base.id()
    }

    /// Test path existence and record the metadata dependency.
    pub fn exists(&mut self, path: &VPath) -> bool {
        let state = self.resolve(path).map(|resolved| resolved.node.state());
        self.record_metadata_dependency(path);
        self.push_effect(Effect::MetadataRead {
            path: path.clone(),
            state,
        });
        state.is_some()
    }

    /// Read virtual metadata without following symbolic links.
    ///
    /// # Errors
    ///
    /// Returns [`VfsError::NotFound`] when `path` is absent.
    pub fn metadata(&mut self, path: &VPath) -> Result<NodeState, VfsError> {
        let state = self.resolve(path).map(|resolved| resolved.node.state());
        self.record_metadata_dependency(path);
        self.push_effect(Effect::MetadataRead {
            path: path.clone(),
            state,
        });
        state.ok_or_else(|| VfsError::NotFound { path: path.clone() })
    }

    /// Read regular-file bytes from virtual state.
    ///
    /// # Errors
    ///
    /// Returns an error for absent paths, non-files, stale lazy content, or blob-store
    /// verification failures.
    pub fn read(&mut self, path: &VPath) -> Result<Vec<u8>, VfsError> {
        let resolved = self
            .resolve(path)
            .ok_or_else(|| VfsError::NotFound { path: path.clone() })?;
        if resolved.node.state.kind() != NodeKind::File {
            return Err(VfsError::NotFile {
                path: path.clone(),
                actual: resolved.node.state.kind(),
            });
        }
        let (blob, bytes) = resolved.node.read(path, self.base.store())?;
        if let Some(origin) = resolved.base_origin {
            let observation = self.read_set.entry(origin).or_default();
            observation.content.get_or_insert(blob);
        }
        self.push_effect(Effect::ContentRead {
            path: path.clone(),
            blob,
        });
        Ok(bytes)
    }

    /// Read an opaque symbolic-link target without following it.
    ///
    /// # Errors
    ///
    /// Returns an error for absent paths, non-links, stale lazy content, or blob errors.
    pub fn read_link(&mut self, path: &VPath) -> Result<Vec<u8>, VfsError> {
        let resolved = self
            .resolve(path)
            .ok_or_else(|| VfsError::NotFound { path: path.clone() })?;
        if resolved.node.state.kind() != NodeKind::Symlink {
            return Err(VfsError::NotSymlink {
                path: path.clone(),
                actual: resolved.node.state.kind(),
            });
        }
        let (blob, bytes) = resolved.node.read(path, self.base.store())?;
        if let Some(origin) = resolved.base_origin {
            let observation = self.read_set.entry(origin).or_default();
            observation.content.get_or_insert(blob);
        }
        self.push_effect(Effect::ContentRead {
            path: path.clone(),
            blob,
        });
        Ok(bytes)
    }

    /// List immediate child paths in canonical order.
    ///
    /// # Errors
    ///
    /// Returns an error when `path` is absent or not a directory.
    pub fn read_dir(&mut self, path: &VPath) -> Result<Vec<VPath>, VfsError> {
        self.require_directory(path)?;
        self.record_metadata_dependency(path);
        let children = self.visible_direct_children(path);
        let digest = self.listing_digest(&children);
        if self.base.node(path).is_some() {
            self.read_set.entry(path.clone()).or_default().directory =
                Some(self.base.directory_digest(path));
        }
        self.push_effect(Effect::DirectoryRead {
            path: path.clone(),
            digest,
        });
        Ok(children)
    }

    /// Create or replace a regular file in the overlay.
    ///
    /// # Errors
    ///
    /// Returns an error when the parent is absent/non-directory, the target is a
    /// directory/link, or immutable blob storage fails.
    pub fn write(&mut self, path: &VPath, bytes: &[u8]) -> Result<(), VfsError> {
        Self::ensure_mutable_path(path)?;
        self.require_parent_directory(path)?;
        let before = self.resolve(path).map(|resolved| resolved.node.state());
        if let Some(state) = before
            && state.kind() != NodeKind::File
        {
            return Err(VfsError::NotFile {
                path: path.clone(),
                actual: state.kind(),
            });
        }

        let blob = self.base.store().put(bytes)?;
        let mode = before.map_or(DEFAULT_FILE_MODE, NodeState::mode);
        let node = Arc::new(SnapshotNode::materialized(
            NodeKind::File,
            blob,
            bytes.len() as u64,
            mode,
        ));
        let after = node.state();
        self.record_write_precondition(path);
        self.overlay.insert(
            path.clone(),
            OverlayEntry::Present(ResolvedNode {
                node,
                base_origin: None,
            }),
        );
        self.push_effect(match before {
            Some(before) => Effect::ModifyContent {
                path: path.clone(),
                before,
                after,
            },
            None => Effect::Create {
                path: path.clone(),
                after,
            },
        });
        Ok(())
    }

    /// Append bytes to a regular file using the same read/write semantics.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::read`] or [`Self::write`].
    pub fn append(&mut self, path: &VPath, suffix: &[u8]) -> Result<(), VfsError> {
        let mut bytes = self.read(path)?;
        bytes.extend_from_slice(suffix);
        self.write(path, &bytes)
    }

    /// Create one empty directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the path exists or its parent is unavailable.
    pub fn mkdir(&mut self, path: &VPath, mode: u32) -> Result<(), VfsError> {
        Self::ensure_mutable_path(path)?;
        self.require_parent_directory(path)?;
        if self.resolve(path).is_some() {
            return Err(VfsError::AlreadyExists { path: path.clone() });
        }
        let node = Arc::new(SnapshotNode::directory(platform_directory_mode(mode)));
        let after = node.state();
        self.record_write_precondition(path);
        self.overlay.insert(
            path.clone(),
            OverlayEntry::Present(ResolvedNode {
                node,
                base_origin: None,
            }),
        );
        self.push_effect(Effect::Create {
            path: path.clone(),
            after,
        });
        Ok(())
    }

    /// Delete one regular file or opaque symbolic link.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is absent or is a directory/root.
    pub fn unlink(&mut self, path: &VPath) -> Result<(), VfsError> {
        Self::ensure_mutable_path(path)?;
        let before = self
            .resolve(path)
            .map(|resolved| resolved.node.state())
            .ok_or_else(|| VfsError::NotFound { path: path.clone() })?;
        if before.kind() == NodeKind::Directory {
            return Err(VfsError::IsDirectory { path: path.clone() });
        }
        self.record_write_precondition(path);
        self.overlay.insert(path.clone(), OverlayEntry::Tombstone);
        self.push_effect(Effect::Delete {
            path: path.clone(),
            before,
        });
        Ok(())
    }

    /// Delete one empty directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is absent, non-directory, non-empty, or root.
    pub fn rmdir(&mut self, path: &VPath) -> Result<(), VfsError> {
        Self::ensure_mutable_path(path)?;
        let before = self.require_directory(path)?;
        let children = self.read_dir(path)?;
        if !children.is_empty() {
            return Err(VfsError::DirectoryNotEmpty { path: path.clone() });
        }
        self.record_write_precondition(path);
        self.overlay.insert(path.clone(), OverlayEntry::Tombstone);
        self.push_effect(Effect::Delete {
            path: path.clone(),
            before,
        });
        Ok(())
    }

    /// Recursively delete a virtual subtree while expanding every affected write.
    ///
    /// # Errors
    ///
    /// Returns an error when the root path is absent or targets the virtual root.
    pub fn remove_tree(&mut self, path: &VPath) -> Result<(), VfsError> {
        Self::ensure_mutable_path(path)?;
        let nodes = self.visible_subtree(path);
        if nodes.is_empty() {
            return Err(VfsError::NotFound { path: path.clone() });
        }
        for (node_path, resolved) in nodes.iter().rev() {
            self.record_write_precondition(node_path);
            self.push_effect(Effect::Delete {
                path: node_path.clone(),
                before: resolved.node.state(),
            });
        }
        for (node_path, _) in nodes {
            self.overlay.insert(node_path, OverlayEntry::Tombstone);
        }
        Ok(())
    }

    /// Move a file, link, or directory subtree without touching the host.
    ///
    /// Existing non-directory destinations are replaced. A directory may replace only
    /// an empty directory. Symlinks are moved as opaque nodes and never followed.
    ///
    /// # Errors
    ///
    /// Returns an error for root/overlapping moves, absent sources, invalid parents, or
    /// incompatible/non-empty destinations.
    pub fn rename(&mut self, from: &VPath, to: &VPath) -> Result<(), VfsError> {
        Self::ensure_mutable_path(from)?;
        Self::ensure_mutable_path(to)?;
        if from == to {
            return Ok(());
        }
        if to.is_within(from) || from.is_within(to) {
            return Err(VfsError::InvalidRename {
                from: from.clone(),
                to: to.clone(),
            });
        }
        self.require_parent_directory(to)?;
        let source = self
            .resolve(from)
            .ok_or_else(|| VfsError::NotFound { path: from.clone() })?;
        let source_state = source.node.state();

        if let Some(destination) = self.resolve(to) {
            let destination_state = destination.node.state();
            match (source_state.kind(), destination_state.kind()) {
                (NodeKind::Directory, NodeKind::Directory)
                    if !self.visible_direct_children(to).is_empty() =>
                {
                    return Err(VfsError::DirectoryNotEmpty { path: to.clone() });
                }
                (NodeKind::Directory, NodeKind::Directory) => {}
                (NodeKind::Directory, _) | (_, NodeKind::Directory) => {
                    return Err(VfsError::RenameTypeMismatch {
                        from: from.clone(),
                        to: to.clone(),
                    });
                }
                _ => {}
            }
            self.record_write_precondition(to);
            self.overlay.insert(to.clone(), OverlayEntry::Tombstone);
        }

        let moving = self.visible_subtree(from);
        for (source_path, _) in &moving {
            self.record_write_precondition(source_path);
        }
        for (source_path, _) in &moving {
            let destination_path =
                source_path
                    .rebase(from, to)?
                    .ok_or_else(|| VfsError::InvalidRename {
                        from: from.clone(),
                        to: to.clone(),
                    })?;
            self.record_write_precondition(&destination_path);
        }

        for (source_path, _) in &moving {
            self.overlay
                .insert(source_path.clone(), OverlayEntry::Tombstone);
        }
        for (source_path, resolved) in moving {
            let destination_path =
                source_path
                    .rebase(from, to)?
                    .ok_or_else(|| VfsError::InvalidRename {
                        from: from.clone(),
                        to: to.clone(),
                    })?;
            self.overlay
                .insert(destination_path, OverlayEntry::Present(resolved));
        }
        self.push_effect(Effect::Rename {
            from: from.clone(),
            to: to.clone(),
            before: source_state,
            after: source_state,
        });
        Ok(())
    }

    /// Produce the exact path-ordered diff between base and final virtual state.
    ///
    /// Descendant closure is expanded for subtree tombstones, so a recursive delete is
    /// never represented as a misleading one-path change.
    ///
    /// # Errors
    ///
    /// Returns an error if changed lazy content cannot be captured and verified.
    pub fn canonical_diff(&self) -> Result<CanonicalDiff, VfsError> {
        let mut candidates = BTreeSet::new();
        let mut expanded_delete_paths = BTreeSet::new();
        for (path, entry) in &self.overlay {
            candidates.insert(path.clone());
            if matches!(entry, OverlayEntry::Tombstone) {
                expanded_delete_paths.extend(self.base.subtree_paths(path));
            }
        }
        candidates.extend(expanded_delete_paths.iter().cloned());

        let candidates: Vec<VPath> = candidates
            .into_iter()
            .filter(|path| !path.is_root())
            .collect();
        let candidate_paths = candidates.len();
        let mut after_states = Vec::with_capacity(candidate_paths);
        let mut materialized_after_bytes = 0_u64;
        for path in &candidates {
            let after = match self.resolve(path) {
                Some(resolved) => Some(resolved.node.materialized_state(path, self.base.store())?),
                None => None,
            };
            materialized_after_bytes =
                materialized_after_bytes.saturating_add(after.map_or(0, NodeState::size));
            after_states.push(after);
        }

        let mut entries = Vec::new();
        // Finish every lazy after-state capture before reading any before-state:
        // rename destinations may share their lazy node with a base source.
        for (path, after) in candidates.into_iter().zip(after_states) {
            let before = self.base.node(&path).map(|node| node.state());
            if before == after {
                continue;
            }
            let kind = classify_diff(before, after);
            entries.push(DiffEntry {
                path,
                before,
                after,
                kind,
            });
        }

        let mut canonical = Vec::new();
        for entry in &entries {
            encode_diff_entry(entry, &mut canonical);
        }
        Ok(CanonicalDiff {
            metrics: CanonicalDiffMetrics {
                candidate_paths,
                expanded_delete_paths: expanded_delete_paths.len(),
                changed_paths: entries.len(),
                materialized_after_bytes,
            },
            entries,
            digest: DiffDigest::digest_canonical(&canonical),
        })
    }

    /// Return the observed effect ledger.
    ///
    /// This excludes host-only evidence capture.
    #[must_use]
    pub fn effects(&self) -> &[EffectEvent] {
        &self.effects
    }

    /// Capture bounded base content before finalizing a reviewable transaction.
    ///
    /// The trusted caller must authorize content reads for every supplied path.
    /// Existing read/write observations and guest effects are not modified. Call
    /// `canonical_diff` afterwards to bind any newly materialized before identities.
    /// Paths exceeding the remaining byte budget retain their lazy stamp.
    ///
    /// # Errors
    ///
    /// Returns an error if base content changed during capture or cannot be read.
    pub fn capture_before_content<'a>(
        &self,
        paths: impl IntoIterator<Item = &'a VPath>,
        maximum: usize,
    ) -> Result<(), VfsError> {
        let mut remaining = u64::try_from(maximum).unwrap_or(u64::MAX);
        for path in paths {
            let Some(node) = self.base.node(path) else {
                continue;
            };
            let state = node.state();
            if state.kind() == NodeKind::Directory || node.is_materialized() {
                continue;
            }
            if state.size() <= remaining {
                node.materialized_state(path, self.base.store())?;
                remaining -= state.size();
            }
        }
        Ok(())
    }

    /// Return base dependencies observed by virtual execution.
    #[must_use]
    pub fn read_set(&self) -> &BTreeMap<VPath, ReadObservation> {
        &self.read_set
    }

    /// Return base preconditions for every virtual write.
    #[must_use]
    pub fn write_set(&self) -> &BTreeMap<VPath, WritePrecondition> {
        &self.write_set
    }

    /// Return bounded transaction-local size metrics.
    #[must_use]
    pub fn metrics(&self) -> VfsMetrics {
        let overlay_bytes = self
            .overlay
            .values()
            .filter_map(|entry| match entry {
                OverlayEntry::Present(resolved) => Some(resolved.node.state().size()),
                OverlayEntry::Tombstone => None,
            })
            .sum();
        VfsMetrics {
            overlay_entries: self.overlay.len(),
            overlay_bytes,
            effect_events: self.effects.len(),
            read_dependencies: self.read_set.len(),
            write_preconditions: self.write_set.len(),
        }
    }

    /// Materialize final virtual state for verification/test harnesses.
    ///
    /// This intentionally scans the full snapshot and is not used by transaction hot
    /// paths; production decisions use [`Self::canonical_diff`].
    ///
    /// # Errors
    ///
    /// Returns an error when visible lazy content cannot be verified.
    pub fn materialized_final_state(&self) -> Result<BTreeMap<VPath, NodeState>, VfsError> {
        let mut candidates: BTreeSet<VPath> = self.base.inner.nodes.keys().cloned().collect();
        candidates.extend(self.overlay.keys().cloned());
        let mut final_state = BTreeMap::new();
        for path in candidates {
            if let Some(resolved) = self.resolve(&path) {
                let state = resolved.node.materialized_state(&path, self.base.store())?;
                final_state.insert(path, state);
            }
        }
        Ok(final_state)
    }

    fn resolve(&self, path: &VPath) -> Option<ResolvedNode> {
        // Read-only transactions have no ancestors to shadow. Do not allocate
        // and probe every parent when the overlay is provably empty.
        if self.overlay.is_empty() {
            return self.base.node(path).map(|node| ResolvedNode {
                node,
                base_origin: Some(path.clone()),
            });
        }
        let mut ancestor = (!path.is_root()).then(|| parent_str(path.as_str()));
        while let Some(current) = ancestor {
            match self.overlay.get(current) {
                Some(OverlayEntry::Tombstone) => return None,
                Some(OverlayEntry::Present(resolved))
                    if resolved.node.state.kind() != NodeKind::Directory =>
                {
                    return None;
                }
                Some(OverlayEntry::Present(_)) | None => {}
            }
            ancestor = (current != ".").then(|| parent_str(current));
        }
        if let Some(entry) = self.overlay.get(path) {
            return match entry {
                OverlayEntry::Present(resolved) => Some(resolved.clone()),
                OverlayEntry::Tombstone => None,
            };
        }
        self.base.node(path).map(|node| ResolvedNode {
            node,
            base_origin: Some(path.clone()),
        })
    }

    fn visible_direct_children(&self, path: &VPath) -> Vec<VPath> {
        if self.overlay.is_empty() {
            return self.base.direct_children(path).collect();
        }
        let mut candidates: BTreeSet<VPath> = self.base.direct_children(path).collect();
        // The slash is part of the lexical lower bound: a/ must not include
        // a-, a., a0 or other prefix siblings. No additional index is retained.
        let prefix = if path.is_root() {
            String::new()
        } else {
            format!("{path}/")
        };
        candidates.extend(
            self.overlay
                .range::<str, _>((Included(prefix.as_str()), Unbounded))
                .take_while(|(candidate, _)| candidate.as_str().starts_with(&prefix))
                .filter(|(candidate, _)| !candidate.as_str()[prefix.len()..].contains('/'))
                .map(|(candidate, _)| candidate.clone()),
        );
        candidates
            .into_iter()
            .filter(|candidate| self.resolve(candidate).is_some())
            .collect()
    }

    fn visible_subtree(&self, path: &VPath) -> Vec<(VPath, ResolvedNode)> {
        let mut candidates: BTreeSet<VPath> = self.base.subtree_paths(path).into_iter().collect();
        candidates.extend(
            self.overlay
                .keys()
                .filter(|candidate| candidate.is_within(path))
                .cloned(),
        );
        candidates
            .into_iter()
            .filter_map(|candidate| self.resolve(&candidate).map(|node| (candidate, node)))
            .collect()
    }

    fn listing_digest(&self, children: &[VPath]) -> DirectoryDigest {
        DirectoryDigest::digest_entries(children.iter().map(|child| {
            (
                child,
                self.resolve(child)
                    .expect("visible child resolves")
                    .node
                    .state(),
            )
        }))
    }

    fn require_directory(&self, path: &VPath) -> Result<NodeState, VfsError> {
        let state = self
            .resolve(path)
            .map(|resolved| resolved.node.state())
            .ok_or_else(|| VfsError::NotFound { path: path.clone() })?;
        if state.kind() != NodeKind::Directory {
            return Err(VfsError::NotDirectory {
                path: path.clone(),
                actual: state.kind(),
            });
        }
        Ok(state)
    }

    fn require_parent_directory(&self, path: &VPath) -> Result<(), VfsError> {
        let parent = path.parent().ok_or(VfsError::RootMutation)?;
        self.require_directory(&parent).map(|_| ())
    }

    fn ensure_mutable_path(path: &VPath) -> Result<(), VfsError> {
        if path.is_root() {
            Err(VfsError::RootMutation)
        } else {
            Ok(())
        }
    }

    fn record_metadata_dependency(&mut self, path: &VPath) {
        let expected = self.base.node(path).map(|node| node.expected_state());
        self.read_set
            .entry(path.clone())
            .or_default()
            .metadata
            .get_or_insert(expected);
    }

    fn record_write_precondition(&mut self, path: &VPath) {
        let expected = self.base.node(path).map(|node| node.expected_state());
        self.write_set
            .entry(path.clone())
            .or_insert(WritePrecondition { expected });
        if let Some(parent) = path.parent() {
            self.record_metadata_dependency(&parent);
        }
    }

    fn push_effect(&mut self, effect: Effect) {
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("effect sequence cannot wrap within a bounded transaction");
        self.effects.push(EffectEvent {
            sequence,
            origin: self.effect_origin,
            effect,
        });
    }
}

fn parent_str(path: &str) -> &str {
    path.rsplit_once('/').map_or(".", |(parent, _)| parent)
}

fn classify_diff(before: Option<NodeState>, after: Option<NodeState>) -> DiffKind {
    match (before, after) {
        (None, Some(_)) => DiffKind::Create,
        (Some(_), None) => DiffKind::Delete,
        (Some(before), Some(after)) if before.content_equivalent(after) => DiffKind::MetadataChange,
        (Some(_), Some(_)) => DiffKind::Modify,
        (None, None) => unreachable!("equal missing states are removed before classification"),
    }
}

fn encode_path(path: &VPath, output: &mut Vec<u8>) {
    let bytes = path.as_str().as_bytes();
    output.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    output.extend_from_slice(bytes);
}

fn encode_optional_state(state: Option<NodeState>, output: &mut Vec<u8>) {
    match state {
        Some(state) => {
            output.push(1);
            state.encode_canonical(output);
        }
        None => output.push(0),
    }
}

fn encode_diff_entry(entry: &DiffEntry, output: &mut Vec<u8>) {
    encode_path(&entry.path, output);
    output.push(match entry.kind {
        DiffKind::Create => 1,
        DiffKind::Delete => 2,
        DiffKind::Modify => 3,
        DiffKind::MetadataChange => 4,
    });
    encode_optional_state(entry.before, output);
    encode_optional_state(entry.after, output);
}

/// Virtual filesystem operation failure.
#[derive(Debug)]
#[non_exhaustive]
pub enum VfsError {
    /// Snapshot access or lazy capture failed.
    Snapshot(SnapshotError),
    /// Immutable blob storage failed.
    Store(BlobStoreError),
    /// A virtual path was invalid during an internal rebase.
    Path(VPathError),
    /// The requested path does not exist in virtual state.
    NotFound {
        /// Missing path.
        path: VPath,
    },
    /// The target already exists.
    AlreadyExists {
        /// Existing path.
        path: VPath,
    },
    /// A directory operation targeted another node kind.
    NotDirectory {
        /// Requested path.
        path: VPath,
        /// Actual kind.
        actual: NodeKind,
    },
    /// A regular-file operation targeted another node kind.
    NotFile {
        /// Requested path.
        path: VPath,
        /// Actual kind.
        actual: NodeKind,
    },
    /// A symlink operation targeted another node kind.
    NotSymlink {
        /// Requested path.
        path: VPath,
        /// Actual kind.
        actual: NodeKind,
    },
    /// A file-only deletion targeted a directory.
    IsDirectory {
        /// Directory path.
        path: VPath,
    },
    /// A non-empty directory cannot be removed/replaced.
    DirectoryNotEmpty {
        /// Non-empty directory.
        path: VPath,
    },
    /// The immutable virtual root cannot be mutated.
    RootMutation,
    /// Source and destination subtrees overlap.
    InvalidRename {
        /// Source root.
        from: VPath,
        /// Destination root.
        to: VPath,
    },
    /// Rename source and destination kinds are incompatible.
    RenameTypeMismatch {
        /// Source path.
        from: VPath,
        /// Destination path.
        to: VPath,
    },
    /// A decoded durable artifact violated canonical diff invariants.
    InvalidCanonicalDiff {
        /// Affected path, when the violation is path-specific.
        path: Option<VPath>,
        /// Stable validation reason.
        reason: &'static str,
    },
}

impl From<SnapshotError> for VfsError {
    fn from(value: SnapshotError) -> Self {
        Self::Snapshot(value)
    }
}

impl From<BlobStoreError> for VfsError {
    fn from(value: BlobStoreError) -> Self {
        Self::Store(value)
    }
}

impl From<VPathError> for VfsError {
    fn from(value: VPathError) -> Self {
        Self::Path(value)
    }
}

impl fmt::Display for VfsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Snapshot(source) => write!(formatter, "snapshot failure: {source}"),
            Self::Store(source) => write!(formatter, "blob store failure: {source}"),
            Self::Path(source) => write!(formatter, "virtual path failure: {source}"),
            Self::NotFound { path } => write!(formatter, "virtual path not found: {path}"),
            Self::AlreadyExists { path } => write!(formatter, "virtual path exists: {path}"),
            Self::NotDirectory { path, actual } => {
                write!(
                    formatter,
                    "virtual path {path} is not a directory ({actual:?})"
                )
            }
            Self::NotFile { path, actual } => {
                write!(formatter, "virtual path {path} is not a file ({actual:?})")
            }
            Self::NotSymlink { path, actual } => {
                write!(
                    formatter,
                    "virtual path {path} is not a symlink ({actual:?})"
                )
            }
            Self::IsDirectory { path } => write!(formatter, "virtual path is a directory: {path}"),
            Self::DirectoryNotEmpty { path } => {
                write!(formatter, "virtual directory is not empty: {path}")
            }
            Self::RootMutation => formatter.write_str("the virtual root cannot be mutated"),
            Self::InvalidRename { from, to } => {
                write!(formatter, "rename subtrees overlap: {from} -> {to}")
            }
            Self::RenameTypeMismatch { from, to } => {
                write!(formatter, "rename type mismatch: {from} -> {to}")
            }
            Self::InvalidCanonicalDiff { path, reason } => match path {
                Some(path) => write!(formatter, "invalid canonical diff at {path}: {reason}"),
                None => write!(formatter, "invalid canonical diff: {reason}"),
            },
        }
    }
}

impl Error for VfsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Snapshot(source) => Some(source),
            Self::Store(source) => Some(source),
            Self::Path(source) => Some(source),
            Self::NotFound { .. }
            | Self::AlreadyExists { .. }
            | Self::NotDirectory { .. }
            | Self::NotFile { .. }
            | Self::NotSymlink { .. }
            | Self::IsDirectory { .. }
            | Self::DirectoryNotEmpty { .. }
            | Self::RootMutation
            | Self::InvalidRename { .. }
            | Self::RenameTypeMismatch { .. }
            | Self::InvalidCanonicalDiff { .. } => None,
        }
    }
}

/// Transaction-local virtual filesystem size counters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VfsMetrics {
    /// Exact overlay entry count.
    pub overlay_entries: usize,
    /// Sum of visible overlay node sizes.
    pub overlay_bytes: u64,
    /// Observed effect count.
    pub effect_events: usize,
    /// Base read-dependency path count.
    pub read_dependencies: usize,
    /// Base write-precondition path count.
    pub write_preconditions: usize,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    use vsh_types::PlatformFileId;

    use super::*;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_store(name: &str) -> (TestDirectory, BlobStore) {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "vsh-vfs-test-{}-{sequence}-{name}",
            std::process::id()
        ));
        let guard = TestDirectory(root.clone());
        let store = BlobStore::open(root).unwrap();
        (guard, store)
    }

    fn path(value: &str) -> VPath {
        VPath::parse(value).unwrap()
    }

    fn stamp(kind: NodeKind, size: usize, identity: u64) -> FileStamp {
        FileStamp {
            kind,
            size: size as u64,
            mode: 0o644,
            mtime_ns: 1_700_000_000_000_000_000,
            ctime_ns: Some(1_700_000_000_000_000_001),
            file_id: PlatformFileId {
                high: 7,
                low: identity,
            },
        }
    }

    fn fixture_snapshot(store: BlobStore) -> BaseSnapshot {
        let mut builder = SnapshotBuilder::new(store);
        builder.add_directory(path("src"), 0o755).unwrap();
        builder.add_directory(path("src/nested"), 0o755).unwrap();
        builder.add_directory(path("empty"), 0o755).unwrap();
        builder
            .add_file(path("src/a.txt"), b"alpha", 0o644)
            .unwrap();
        builder
            .add_file(path("src/nested/b.txt"), b"beta", 0o600)
            .unwrap();
        builder
            .add_symlink(path("opaque-link"), b"../../outside", 0o777)
            .unwrap();
        builder.build().unwrap()
    }

    #[test]
    fn public_snapshot_errors_have_stable_messages_and_sources() {
        let expected = stamp(NodeKind::File, 1, 1);
        let changed = FileStamp {
            size: 2,
            ..expected
        };
        let blob_error = || BlobStoreError::Io {
            operation: "read",
            path: PathBuf::from("blob"),
            source: io::Error::other("test"),
        };
        let snapshot_errors = [
            SnapshotError::Store(blob_error()),
            SnapshotError::DuplicatePath { path: path("file") },
            SnapshotError::MissingParent {
                path: path("dir/file"),
                parent: path("dir"),
            },
            SnapshotError::ParentNotDirectory {
                path: path("dir/file"),
                parent: path("dir"),
            },
            SnapshotError::LazyDirectory { path: path("dir") },
            SnapshotError::ExpectedDirectoryStamp {
                path: path("file"),
                stamp: expected,
            },
            SnapshotError::ContentUnavailable {
                path: path("dir"),
                kind: NodeKind::Directory,
            },
            SnapshotError::ContentLoad {
                path: path("file"),
                source: ContentLoadError::new("test"),
            },
            SnapshotError::StaleContent {
                path: path("file"),
                expected: Box::new(expected),
                before: Box::new(expected),
                after: Box::new(changed),
            },
            SnapshotError::ContentSizeMismatch {
                path: path("file"),
                expected: 1,
                actual: 2,
            },
            SnapshotError::LazyStatePoisoned { path: path("file") },
        ];
        for error in snapshot_errors {
            assert!(!error.to_string().is_empty());
            assert_eq!(
                Error::source(&error).is_some(),
                matches!(
                    error,
                    SnapshotError::Store(_) | SnapshotError::ContentLoad { .. }
                )
            );
        }
    }

    #[test]
    fn public_vfs_errors_have_stable_messages_and_sources() {
        let blob_error = || BlobStoreError::Io {
            operation: "read",
            path: PathBuf::from("blob"),
            source: io::Error::other("test"),
        };
        let vfs_errors = [
            VfsError::Snapshot(SnapshotError::DuplicatePath { path: path("file") }),
            VfsError::Store(blob_error()),
            VfsError::Path(VPath::parse("").unwrap_err()),
            VfsError::NotFound { path: path("file") },
            VfsError::AlreadyExists { path: path("file") },
            VfsError::NotDirectory {
                path: path("file"),
                actual: NodeKind::File,
            },
            VfsError::NotFile {
                path: path("dir"),
                actual: NodeKind::Directory,
            },
            VfsError::NotSymlink {
                path: path("file"),
                actual: NodeKind::File,
            },
            VfsError::IsDirectory { path: path("dir") },
            VfsError::DirectoryNotEmpty { path: path("dir") },
            VfsError::RootMutation,
            VfsError::InvalidRename {
                from: path("dir"),
                to: path("dir/child"),
            },
            VfsError::RenameTypeMismatch {
                from: path("file"),
                to: path("dir"),
            },
            VfsError::InvalidCanonicalDiff {
                path: Some(path("file")),
                reason: "test",
            },
            VfsError::InvalidCanonicalDiff {
                path: None,
                reason: "test",
            },
        ];
        for error in vfs_errors {
            assert!(!error.to_string().is_empty());
            assert_eq!(
                Error::source(&error).is_some(),
                matches!(
                    error,
                    VfsError::Snapshot(_) | VfsError::Store(_) | VfsError::Path(_)
                )
            );
        }
    }

    #[test]
    fn snapshot_identity_is_insertion_order_and_store_independent() {
        let (_first_guard, first_store) = test_store("snapshot-a");
        let (_second_guard, second_store) = test_store("snapshot-b");

        let mut first = SnapshotBuilder::new(first_store);
        first.add_directory(path("dir"), 0o755).unwrap();
        first.add_file(path("dir/file"), b"same", 0o640).unwrap();

        let mut second = SnapshotBuilder::new(second_store);
        second.add_file(path("dir/file"), b"same", 0o640).unwrap();
        second.add_directory(path("dir"), 0o755).unwrap();

        let first = first.build().unwrap();
        let second = second.build().unwrap();
        assert_eq!(first.id(), second.id());
        assert_eq!(first.len(), 3);
        assert!(!first.is_empty());
    }

    #[test]
    fn snapshot_rejects_missing_or_non_directory_parents() {
        let (_guard, store) = test_store("invalid-parent");
        let mut missing = SnapshotBuilder::new(store.clone());
        missing.add_file(path("missing/file"), b"x", 0o644).unwrap();
        assert!(matches!(
            missing.build(),
            Err(SnapshotError::MissingParent { .. })
        ));

        let mut non_directory = SnapshotBuilder::new(store);
        non_directory.add_file(path("file"), b"x", 0o644).unwrap();
        non_directory
            .add_file(path("file/child"), b"y", 0o644)
            .unwrap();
        assert!(matches!(
            non_directory.build(),
            Err(SnapshotError::ParentNotDirectory { .. })
        ));
    }

    #[test]
    fn lazy_content_is_captured_once_and_then_blob_backed() {
        let (_guard, store) = test_store("lazy-once");
        let expected = stamp(NodeKind::File, 5, 41);
        let calls = Arc::new(AtomicUsize::new(0));
        let loader_calls = Arc::clone(&calls);
        let mut builder = SnapshotBuilder::new(store);
        builder
            .add_lazy(path("lazy.txt"), expected, move |stamp| {
                loader_calls.fetch_add(1, Ordering::Relaxed);
                Ok(CapturedContent {
                    bytes: b"hello".to_vec(),
                    before: stamp,
                    after: stamp,
                })
            })
            .unwrap();
        let snapshot = builder.build().unwrap();
        assert_eq!(
            snapshot.metrics(),
            SnapshotMetrics {
                node_count: 2,
                lazy_content_nodes: 1,
                materialized_content_nodes: 0,
            }
        );

        let mut vfs = VirtualFs::new(snapshot.clone());
        assert_eq!(vfs.read(&path("lazy.txt")).unwrap(), b"hello");
        assert_eq!(vfs.read(&path("lazy.txt")).unwrap(), b"hello");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(snapshot.metrics().materialized_content_nodes, 1);
        assert!(vfs.read_set()[&path("lazy.txt")].content.is_some());
    }

    #[test]
    fn concurrent_snapshot_readers_share_one_lazy_capture() {
        let (_guard, store) = test_store("lazy-concurrent");
        let expected = stamp(NodeKind::File, 5, 43);
        let calls = Arc::new(AtomicUsize::new(0));
        let loader_calls = Arc::clone(&calls);
        let mut builder = SnapshotBuilder::new(store);
        builder
            .add_lazy(path("lazy.txt"), expected, move |stamp| {
                loader_calls.fetch_add(1, Ordering::Relaxed);
                Ok(CapturedContent {
                    bytes: b"hello".to_vec(),
                    before: stamp,
                    after: stamp,
                })
            })
            .unwrap();
        let snapshot = builder.build().unwrap();
        let barrier = Arc::new(Barrier::new(8));
        let mut threads = Vec::new();
        for _ in 0..8 {
            let worker_snapshot = snapshot.clone();
            let worker_barrier = Arc::clone(&barrier);
            threads.push(std::thread::spawn(move || {
                let mut vfs = VirtualFs::new(worker_snapshot);
                worker_barrier.wait();
                vfs.read(&path("lazy.txt")).unwrap()
            }));
        }

        for thread in threads {
            assert_eq!(thread.join().unwrap(), b"hello");
        }
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn lazy_capture_fails_closed_on_stamp_or_size_drift() {
        let (_guard, store) = test_store("lazy-stale");
        let expected = stamp(NodeKind::File, 5, 42);
        let mut changed = expected;
        changed.mtime_ns += 1;
        let mut builder = SnapshotBuilder::new(store);
        builder
            .add_lazy(path("stale.txt"), expected, move |_| {
                Ok(CapturedContent {
                    bytes: b"hello".to_vec(),
                    before: expected,
                    after: changed,
                })
            })
            .unwrap();
        let mut vfs = VirtualFs::new(builder.build().unwrap());
        assert!(matches!(
            vfs.read(&path("stale.txt")),
            Err(VfsError::Snapshot(SnapshotError::StaleContent { .. }))
        ));

        let (_guard, store) = test_store("lazy-size");
        let mut builder = SnapshotBuilder::new(store);
        builder
            .add_lazy(path("size.txt"), expected, move |stamp| {
                Ok(CapturedContent {
                    bytes: b"shorter?".to_vec(),
                    before: stamp,
                    after: stamp,
                })
            })
            .unwrap();
        let mut vfs = VirtualFs::new(builder.build().unwrap());
        assert!(matches!(
            vfs.read(&path("size.txt")),
            Err(VfsError::Snapshot(
                SnapshotError::ContentSizeMismatch { .. }
            ))
        ));
    }

    #[test]
    fn virtual_operations_never_mutate_the_base_and_emit_exact_diff() {
        let (_guard, store) = test_store("operations");
        let snapshot = fixture_snapshot(store);
        let pristine = VirtualFs::new(snapshot.clone())
            .materialized_final_state()
            .unwrap();
        let mut vfs = VirtualFs::new(snapshot.clone());

        assert_eq!(vfs.read(&path("src/a.txt")).unwrap(), b"alpha");
        vfs.append(&path("src/a.txt"), b"!").unwrap();
        vfs.mkdir(&path("generated"), 0o755).unwrap();
        vfs.write(&path("generated/out.txt"), b"output").unwrap();
        vfs.rename(&path("src/nested"), &path("moved")).unwrap();
        vfs.unlink(&path("opaque-link")).unwrap();

        let diff = vfs.canonical_diff().unwrap();
        assert_eq!(diff, vfs.canonical_diff().unwrap());
        assert!(
            diff.entries()
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );

        let mut applied = pristine;
        apply_diff(&mut applied, &diff);
        assert_eq!(applied, vfs.materialized_final_state().unwrap());

        let untouched = VirtualFs::new(snapshot).materialized_final_state().unwrap();
        assert_eq!(untouched[&path("src/a.txt")].size(), 5);
        assert!(untouched.contains_key(&path("opaque-link")));
        assert!(!untouched.contains_key(&path("generated")));
    }

    #[test]
    fn recursive_delete_expands_full_descendant_closure() {
        let (_guard, store) = test_store("delete-closure");
        let snapshot = fixture_snapshot(store);
        let mut vfs = VirtualFs::new(snapshot);
        vfs.remove_tree(&path("src")).unwrap();

        let diff = vfs.canonical_diff().unwrap();
        let deleted: Vec<&str> = diff
            .entries()
            .iter()
            .map(|entry| {
                assert_eq!(entry.kind, DiffKind::Delete);
                entry.path.as_str()
            })
            .collect();
        assert_eq!(
            deleted,
            ["src", "src/a.txt", "src/nested", "src/nested/b.txt"]
        );
        assert_eq!(
            diff.metrics(),
            CanonicalDiffMetrics {
                candidate_paths: 4,
                expanded_delete_paths: 4,
                changed_paths: 4,
                materialized_after_bytes: 0,
            }
        );
        assert_eq!(vfs.write_set().len(), 4);
    }

    #[test]
    fn subtree_expansion_is_component_aware_not_lexical_prefix_based() {
        let (_guard, store) = test_store("subtree-order");
        let mut builder = SnapshotBuilder::new(store);
        builder.add_directory(path("a"), 0o755).unwrap();
        builder.add_file(path("a/in"), b"in", 0o644).unwrap();
        builder
            .add_file(path("a-foreign"), b"foreign", 0o644)
            .unwrap();
        builder.add_file(path("a.other"), b"other", 0o644).unwrap();
        let mut vfs = VirtualFs::new(builder.build().unwrap());

        vfs.remove_tree(&path("a")).unwrap();
        let diff = vfs.canonical_diff().unwrap();
        let paths: Vec<&str> = diff
            .entries()
            .iter()
            .map(|entry| entry.path.as_str())
            .collect();
        assert_eq!(paths, ["a", "a/in"]);
        assert!(vfs.exists(&path("a-foreign")));
        assert!(vfs.exists(&path("a.other")));
    }

    #[test]
    fn subtree_whiteouts_hide_old_and_overlay_descendants_after_recreation() {
        let (_guard, store) = test_store("whiteout");
        let snapshot = fixture_snapshot(store);
        let mut base_model = VirtualFs::new(snapshot.clone())
            .materialized_final_state()
            .unwrap();
        let mut vfs = VirtualFs::new(snapshot);

        vfs.write(&path("src/overlay.txt"), b"overlay").unwrap();
        vfs.remove_tree(&path("src")).unwrap();
        assert!(!vfs.exists(&path("src/a.txt")));
        assert!(!vfs.exists(&path("src/overlay.txt")));

        vfs.mkdir(&path("src"), 0o755).unwrap();
        assert!(vfs.read_dir(&path("src")).unwrap().is_empty());
        vfs.write(&path("src/fresh.txt"), b"fresh").unwrap();
        assert_eq!(vfs.read_dir(&path("src")).unwrap(), [path("src/fresh.txt")]);

        let diff = vfs.canonical_diff().unwrap();
        apply_diff(&mut base_model, &diff);
        assert_eq!(base_model, vfs.materialized_final_state().unwrap());
        assert!(!base_model.contains_key(&path("src/a.txt")));
        assert!(!base_model.contains_key(&path("src/overlay.txt")));
    }

    #[test]
    fn overlay_child_ranges_match_full_scan_with_prefix_siblings() {
        let (_guard, store) = test_store("overlay-child-ranges");
        let mut vfs = VirtualFs::new(SnapshotBuilder::new(store).build().unwrap());
        for directory in ["a", "a!", "a-", "a.", "a0", "á", "a/nested"] {
            vfs.mkdir(&path(directory), 0o755).unwrap();
            vfs.write(&path(&format!("{directory}/file")), b"x")
                .unwrap();
        }
        vfs.unlink(&path("a!/file")).unwrap();
        for directory in [".", "a", "a!", "a-", "a.", "a0", "á", "a/nested"] {
            let directory = path(directory);
            let expected: Vec<VPath> = vfs
                .overlay
                .keys()
                .filter(|candidate| candidate.parent().as_ref() == Some(&directory))
                .filter(|candidate| vfs.resolve(candidate).is_some())
                .cloned()
                .collect();
            assert_eq!(
                vfs.visible_direct_children(&directory),
                expected,
                "{directory}"
            );
        }
    }

    #[test]
    fn empty_overlay_resolution_and_later_whiteouts_preserve_visibility() {
        let (_guard, store) = test_store("empty-overlay-resolve");
        let mut vfs = VirtualFs::new(fixture_snapshot(store));
        assert!(vfs.overlay.is_empty());
        assert!(vfs.resolve(&VPath::root()).is_some());
        assert!(vfs.resolve(&path("src/nested/b.txt")).is_some());
        assert!(vfs.resolve(&path("missing/child")).is_none());
        vfs.remove_tree(&path("src")).unwrap();
        assert!(vfs.resolve(&path("src/nested/b.txt")).is_none());
        vfs.write(&path("src"), b"now a file").unwrap();
        assert!(vfs.resolve(&path("src")).is_some());
        assert!(vfs.resolve(&path("src/nested/b.txt")).is_none());
    }

    #[test]
    fn rename_whiteouts_hide_source_descendants_if_source_is_recreated() {
        let (_guard, store) = test_store("rename-whiteout");
        let mut vfs = VirtualFs::new(fixture_snapshot(store));
        vfs.write(&path("src/overlay.txt"), b"overlay").unwrap();
        vfs.rename(&path("src"), &path("destination")).unwrap();

        assert!(!vfs.exists(&path("src/overlay.txt")));
        assert_eq!(
            vfs.read(&path("destination/overlay.txt")).unwrap(),
            b"overlay"
        );
        vfs.mkdir(&path("src"), 0o755).unwrap();
        assert!(vfs.read_dir(&path("src")).unwrap().is_empty());
        assert!(vfs.exists(&path("destination/nested/b.txt")));
    }

    #[test]
    fn one_file_diff_in_large_snapshot_only_compares_touched_path() {
        let (_guard, store) = test_store("touched-scaling");
        let blob = store.put(b"x").unwrap();
        let mut builder = SnapshotBuilder::new(store);
        builder.add_directory(path("bulk"), 0o755).unwrap();
        for index in 0..10_000 {
            builder
                .insert(
                    path(&format!("bulk/file-{index:05}")),
                    SnapshotNode::materialized(NodeKind::File, blob, 1, 0o644),
                )
                .unwrap();
        }
        let snapshot = builder.build().unwrap();
        assert_eq!(snapshot.len(), 10_002);

        let mut vfs = VirtualFs::new(snapshot);
        vfs.write(&path("bulk/file-05000"), b"changed").unwrap();
        let diff = vfs.canonical_diff().unwrap();

        assert_eq!(
            diff.metrics(),
            CanonicalDiffMetrics {
                candidate_paths: 1,
                expanded_delete_paths: 0,
                changed_paths: 1,
                materialized_after_bytes: 7,
            }
        );
    }

    #[test]
    fn reads_writes_and_effects_are_transaction_local_and_ordered() {
        let (_guard, store) = test_store("ledger");
        let mut vfs = VirtualFs::new(fixture_snapshot(store));

        vfs.read(&path("src/a.txt")).unwrap();
        vfs.read_dir(&path("src")).unwrap();
        assert!(!vfs.exists(&path("missing")));
        vfs.write(&path("created.txt"), b"new").unwrap();

        assert!(vfs.read_set()[&path("src/a.txt")].content.is_some());
        assert!(vfs.read_set()[&path("src")].directory.is_some());
        assert_eq!(vfs.read_set()[&path("missing")].metadata, Some(None));
        assert_eq!(
            vfs.write_set()[&path("created.txt")],
            WritePrecondition { expected: None }
        );
        assert!(
            vfs.effects()
                .iter()
                .enumerate()
                .all(|(index, event)| event.sequence == index as u64)
        );
        assert_eq!(vfs.metrics().effect_events, vfs.effects().len());
    }

    #[test]
    fn symlinks_are_opaque_and_never_followed() {
        let (_guard, store) = test_store("symlink");
        let mut vfs = VirtualFs::new(fixture_snapshot(store));
        assert_eq!(
            vfs.read_link(&path("opaque-link")).unwrap(),
            b"../../outside"
        );
        assert!(matches!(
            vfs.read(&path("opaque-link")),
            Err(VfsError::NotFile {
                actual: NodeKind::Symlink,
                ..
            })
        ));
        assert!(matches!(
            vfs.write(&path("opaque-link/child"), b"blocked"),
            Err(VfsError::NotDirectory {
                actual: NodeKind::Symlink,
                ..
            })
        ));
    }

    #[test]
    fn no_op_sequences_have_empty_canonical_diffs() {
        let (_guard, store) = test_store("no-op");
        let snapshot = fixture_snapshot(store);
        let mut vfs = VirtualFs::new(snapshot);
        vfs.write(&path("temporary"), b"x").unwrap();
        vfs.unlink(&path("temporary")).unwrap();
        vfs.write(&path("src/a.txt"), b"alpha").unwrap();

        assert!(vfs.canonical_diff().unwrap().is_empty());
        assert!(!vfs.effects().is_empty());
    }

    #[test]
    fn lazy_rename_diff_is_stable_across_repeated_generation() {
        let (_guard, store) = test_store("lazy-rename");
        let expected = stamp(NodeKind::File, 5, 91);
        let mut builder = SnapshotBuilder::new(store);
        builder
            .add_lazy(path("z.txt"), expected, move |stamp| {
                Ok(CapturedContent {
                    bytes: b"hello".to_vec(),
                    before: stamp,
                    after: stamp,
                })
            })
            .unwrap();
        let mut vfs = VirtualFs::new(builder.build().unwrap());
        vfs.rename(&path("z.txt"), &path("a.txt")).unwrap();

        let first = vfs.canonical_diff().unwrap();
        let second = vfs.canonical_diff().unwrap();
        assert_eq!(first, second);
        assert_eq!(first.entries().len(), 2);
    }

    #[test]
    fn generated_operation_sequences_replay_through_diff_to_same_state() {
        for seed in 0..128 {
            let (_guard, store) = test_store("model-property");
            let base = property_snapshot(store);
            let mut expected = VirtualFs::new(base.clone())
                .materialized_final_state()
                .unwrap();
            let mut vfs = VirtualFs::new(base);
            let mut random = Lcg::new(seed);

            for _ in 0..48 {
                apply_generated_operation(&mut vfs, &mut random);
            }

            let diff = vfs.canonical_diff().unwrap();
            apply_diff(&mut expected, &diff);
            assert_eq!(
                expected,
                vfs.materialized_final_state().unwrap(),
                "seed {seed}"
            );
            assert_eq!(diff, vfs.canonical_diff().unwrap(), "seed {seed}");
        }
    }

    fn property_snapshot(store: BlobStore) -> BaseSnapshot {
        let mut builder = SnapshotBuilder::new(store);
        builder.add_directory(path("a"), 0o755).unwrap();
        builder.add_directory(path("b"), 0o755).unwrap();
        builder.add_directory(path("a/sub"), 0o755).unwrap();
        builder.add_file(path("a/f0"), b"0", 0o644).unwrap();
        builder.add_file(path("a/f1"), b"1", 0o644).unwrap();
        builder
            .add_file(path("a/sub/deep"), b"deep", 0o600)
            .unwrap();
        builder.add_file(path("b/f2"), b"2", 0o644).unwrap();
        builder.build().unwrap()
    }

    fn apply_diff(state: &mut BTreeMap<VPath, NodeState>, diff: &CanonicalDiff) {
        for entry in diff.entries() {
            match entry.after {
                Some(after) => {
                    state.insert(entry.path.clone(), after);
                }
                None => {
                    state.remove(&entry.path);
                }
            }
        }
    }

    fn apply_generated_operation(vfs: &mut VirtualFs, random: &mut Lcg) {
        const FILES: [&str; 6] = ["a/f0", "a/f1", "a/sub/deep", "b/f2", "c/f3", "d/f4"];
        const DIRECTORIES: [&str; 4] = ["a/sub", "c", "d", "b/sub"];
        const RENAMES: [(&str, &str); 6] = [
            ("a/f0", "b/f0"),
            ("b/f0", "a/f0"),
            ("a/sub", "b/sub"),
            ("b/sub", "a/sub"),
            ("c", "d"),
            ("d", "c"),
        ];

        match random.next() % 7 {
            0 => {
                let target = path(FILES[random.index(FILES.len())]);
                let payload = random.next().to_le_bytes();
                let _ = vfs.write(&target, &payload);
            }
            1 => {
                let target = path(DIRECTORIES[random.index(DIRECTORIES.len())]);
                let _ = vfs.mkdir(&target, 0o755);
            }
            2 => {
                let target = path(FILES[random.index(FILES.len())]);
                let _ = vfs.unlink(&target);
            }
            3 => {
                let target = path(DIRECTORIES[random.index(DIRECTORIES.len())]);
                let _ = vfs.rmdir(&target);
            }
            4 => {
                let target = path(DIRECTORIES[random.index(DIRECTORIES.len())]);
                let _ = vfs.remove_tree(&target);
            }
            5 => {
                let (from, to) = RENAMES[random.index(RENAMES.len())];
                let _ = vfs.rename(&path(from), &path(to));
            }
            6 => {
                let target = path(FILES[random.index(FILES.len())]);
                let _ = vfs.append(&target, b"+");
            }
            _ => unreachable!(),
        }
    }

    struct Lcg(u64);

    impl Lcg {
        fn new(seed: u64) -> Self {
            Self(seed ^ 0x9e37_79b9_7f4a_7c15)
        }

        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            self.0
        }

        fn index(&mut self, len: usize) -> usize {
            usize::try_from(self.next() % len as u64).unwrap()
        }
    }
}
