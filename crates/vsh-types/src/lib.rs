//! Security-sensitive value types shared by the VSH Rust core.

use std::borrow::{Borrow, Cow};
use std::error::Error;
use std::fmt;
use std::str::FromStr;

/// A normalized, workspace-relative virtual path.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct VPath(String);

// Canonical string ordering is exactly VPath ordering. Borrowed lookups let
// internal indexes inspect ancestors/prefix ranges without constructing paths.
impl Borrow<str> for VPath {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl VPath {
    /// Return the canonical virtual workspace root.
    #[must_use]
    pub fn root() -> Self {
        Self(".".to_owned())
    }

    /// Parse and normalize a portable virtual path.
    ///
    /// Both slash styles are treated as separators so a path accepted on one host
    /// cannot become an escape on another. Parent components may simplify a path but
    /// may never escape the virtual root.
    ///
    /// # Errors
    ///
    /// Returns [`VPathError`] when the input is empty, absolute, contains a NUL byte or
    /// platform prefix, or would escape the virtual root during normalization.
    pub fn parse(input: &str) -> Result<Self, VPathError> {
        if input.is_empty() {
            return Err(VPathError::Empty);
        }
        if input.contains('\0') {
            return Err(VPathError::NulByte);
        }

        let portable = if input.contains('\\') {
            Cow::Owned(input.replace('\\', "/"))
        } else {
            Cow::Borrowed(input)
        };
        if portable.starts_with('/') {
            return Err(VPathError::Absolute);
        }

        let first = portable.split('/').next().unwrap_or_default();
        if is_windows_prefix(first) {
            return Err(VPathError::PlatformPrefix);
        }

        // Snapshot traversal and guest calls predominantly use canonical paths.
        // Keep the same validation, but avoid a component vector and re-join.
        if portable == "."
            || portable
                .split('/')
                .all(|component| !matches!(component, "" | "." | ".."))
        {
            return Ok(Self(portable.into_owned()));
        }

        let mut components = Vec::new();
        for component in portable.split('/') {
            match component {
                "" | "." => {}
                ".." => {
                    if components.pop().is_none() {
                        return Err(VPathError::EscapesRoot);
                    }
                }
                value => components.push(value),
            }
        }

        let normalized = if components.is_empty() {
            ".".to_owned()
        } else {
            components.join("/")
        };
        Ok(Self(normalized))
    }

    /// Return the canonical slash-separated representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Return whether this path denotes the virtual workspace root.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.0 == "."
    }

    /// Return the normalized parent, or `None` for the virtual root.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        match self.0.rsplit_once('/') {
            Some((parent, _)) => Some(Self(parent.to_owned())),
            None => Some(Self(".".to_owned())),
        }
    }

    /// Return the final path component, or `None` for the virtual root.
    #[must_use]
    pub fn file_name(&self) -> Option<&str> {
        (!self.is_root()).then(|| self.0.rsplit('/').next().unwrap_or(self.as_str()))
    }

    /// Join and normalize a relative child path.
    ///
    /// # Errors
    ///
    /// Returns [`VPathError`] if `child` is invalid or the combined path escapes the
    /// virtual root.
    pub fn join(&self, child: &str) -> Result<Self, VPathError> {
        let combined = if self.is_root() {
            child.to_owned()
        } else {
            format!("{}/{child}", self.as_str())
        };
        Self::parse(&combined)
    }

    /// Return whether this path is equal to or below `ancestor`.
    #[must_use]
    pub fn is_within(&self, ancestor: &Self) -> bool {
        ancestor.is_root()
            || self == ancestor
            || self
                .0
                .strip_prefix(ancestor.as_str())
                .is_some_and(|suffix| suffix.starts_with('/'))
    }

    /// Return the normalized relative suffix below `ancestor`.
    #[must_use]
    pub fn relative_to<'a>(&'a self, ancestor: &Self) -> Option<&'a str> {
        if self == ancestor {
            return Some("");
        }
        if ancestor.is_root() {
            return Some(self.as_str());
        }
        self.0
            .strip_prefix(ancestor.as_str())
            .and_then(|suffix| suffix.strip_prefix('/'))
    }

    /// Rebase this path from one virtual subtree to another.
    ///
    /// # Errors
    ///
    /// Returns [`VPathError`] if the rebased path would be invalid.
    pub fn rebase(&self, from: &Self, to: &Self) -> Result<Option<Self>, VPathError> {
        let Some(suffix) = self.relative_to(from) else {
            return Ok(None);
        };
        if suffix.is_empty() {
            return Ok(Some(to.clone()));
        }
        to.join(suffix).map(Some)
    }
}

impl fmt::Display for VPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl TryFrom<&str> for VPath {
    type Error = VPathError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

fn is_windows_prefix(component: &str) -> bool {
    let bytes = component.as_bytes();
    bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

/// A reason a virtual path was rejected.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum VPathError {
    /// An empty string has no unambiguous virtual-path meaning.
    Empty,
    /// The path is absolute.
    Absolute,
    /// Normalization would leave the virtual workspace root.
    EscapesRoot,
    /// The path contains a NUL byte.
    NulByte,
    /// The path starts with a platform-specific absolute prefix such as `C:`.
    PlatformPrefix,
}

impl fmt::Display for VPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "virtual path must not be empty",
            Self::Absolute => "virtual path must be relative",
            Self::EscapesRoot => "virtual path escapes the workspace root",
            Self::NulByte => "virtual path contains a NUL byte",
            Self::PlatformPrefix => "virtual path contains an absolute platform prefix",
        })
    }
}

impl Error for VPathError {}

macro_rules! digest_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub struct $name([u8; 32]);

        impl $name {
            #[doc = "Construct an identifier from its canonical 32 bytes."]
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }

            #[doc = "Return the canonical identifier bytes."]
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, formatter)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                for byte in self.0 {
                    write!(formatter, "{byte:02x}")?;
                }
                Ok(())
            }
        }

        impl FromStr for $name {
            type Err = ParseDigestError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                decode_digest(value).map(Self::from_bytes)
            }
        }
    };
}

/// A canonical 32-byte lowercase/uppercase hexadecimal identifier was malformed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ParseDigestError {
    /// The textual form was not exactly 64 ASCII bytes.
    InvalidLength {
        /// Observed byte length.
        observed: usize,
    },
    /// One byte was not an ASCII hexadecimal digit.
    InvalidHex {
        /// Zero-based byte offset of the invalid digit.
        index: usize,
    },
}

impl fmt::Display for ParseDigestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { observed } => {
                write!(
                    formatter,
                    "digest must be 64 hexadecimal bytes, got {observed}"
                )
            }
            Self::InvalidHex { index } => {
                write!(
                    formatter,
                    "digest contains non-hexadecimal byte at index {index}"
                )
            }
        }
    }
}

impl Error for ParseDigestError {}

fn decode_digest(value: &str) -> Result<[u8; 32], ParseDigestError> {
    let bytes = value.as_bytes();
    if bytes.len() != 64 {
        return Err(ParseDigestError::InvalidLength {
            observed: bytes.len(),
        });
    }
    let mut decoded = [0_u8; 32];
    for (index, pair) in bytes.chunks_exact(2).enumerate() {
        let high =
            decode_hex_digit(pair[0]).ok_or(ParseDigestError::InvalidHex { index: index * 2 })?;
        let low = decode_hex_digit(pair[1]).ok_or(ParseDigestError::InvalidHex {
            index: index * 2 + 1,
        })?;
        decoded[index] = (high << 4) | low;
    }
    Ok(decoded)
}

const fn decode_hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

digest_id!(BlobId, "The content digest of an immutable blob.");
digest_id!(
    SnapshotId,
    "The digest identity of an immutable base snapshot."
);
digest_id!(
    TransactionId,
    "The digest identity of an exact VSH transaction."
);
digest_id!(
    DiffDigest,
    "The digest identity of one canonical virtual filesystem diff."
);
digest_id!(
    DirectoryDigest,
    "The digest identity of one observed directory listing."
);
digest_id!(
    ProgramDigest,
    "The digest identity of the exact untrusted program source."
);
digest_id!(
    ReadSetDigest,
    "The digest identity of one canonical transaction read set."
);
digest_id!(
    WriteSetDigest,
    "The digest identity of one canonical transaction write set."
);
digest_id!(
    PolicyDigest,
    "The digest identity of one deterministic policy configuration."
);
digest_id!(
    RuntimeConfigDigest,
    "The digest identity of security-relevant runtime configuration."
);
digest_id!(
    IntentDigest,
    "The digest identity of transaction intent supplied out of band."
);
digest_id!(
    PrincipalId,
    "The opaque digest identity of an independent approval principal."
);
digest_id!(
    ApprovalId,
    "The digest identity of one exact bounded approval grant."
);
digest_id!(
    HookId,
    "The opaque digest identity of one configured commit hook."
);
digest_id!(
    RequestEventId,
    "The digest identity of one exact commit-hook request event."
);

impl BlobId {
    /// Hash immutable blob bytes with BLAKE3.
    #[must_use]
    pub fn digest(bytes: &[u8]) -> Self {
        Self::from_bytes(*blake3::hash(bytes).as_bytes())
    }
}

impl SnapshotId {
    /// Hash a canonical snapshot manifest with a VSH domain separator.
    #[must_use]
    pub fn digest_manifest(canonical_manifest: &[u8]) -> Self {
        Self::from_bytes(domain_hash(b"snapshot-v1", canonical_manifest))
    }
}

impl DiffDigest {
    /// Hash a canonical diff encoding with a VSH domain separator.
    #[must_use]
    pub fn digest_canonical(canonical_diff: &[u8]) -> Self {
        Self::from_bytes(domain_hash(b"diff-v1", canonical_diff))
    }
}

impl DirectoryDigest {
    /// Hash a canonical directory-listing encoding with a VSH domain separator.
    #[must_use]
    pub fn digest_canonical(canonical_listing: &[u8]) -> Self {
        Self::from_bytes(domain_hash(b"directory-v1", canonical_listing))
    }

    /// Hash path-ordered direct children using VSH's canonical listing encoding.
    ///
    /// Callers must provide entries in canonical [`VPath`] order. Keeping this codec in
    /// the shared type crate ensures snapshot capture, virtual reads, and trusted host
    /// revalidation cannot silently diverge.
    #[must_use]
    pub fn digest_entries<'a>(entries: impl IntoIterator<Item = (&'a VPath, NodeState)>) -> Self {
        let mut canonical = Vec::new();
        for (path, state) in entries {
            encode_vpath(path, &mut canonical);
            state.encode_canonical(&mut canonical);
        }
        Self::digest_canonical(&canonical)
    }
}

impl ProgramDigest {
    /// Hash exact program UTF-8 bytes with a VSH domain separator.
    #[must_use]
    pub fn digest_source(source: &str) -> Self {
        Self::from_bytes(domain_hash(b"program-v1", source.as_bytes()))
    }
}

impl ReadSetDigest {
    /// Hash a canonical read-set encoding with a VSH domain separator.
    #[must_use]
    pub fn digest_canonical(canonical_read_set: &[u8]) -> Self {
        Self::from_bytes(domain_hash(b"read-set-v1", canonical_read_set))
    }
}

impl WriteSetDigest {
    /// Hash a canonical write-set encoding with a VSH domain separator.
    #[must_use]
    pub fn digest_canonical(canonical_write_set: &[u8]) -> Self {
        Self::from_bytes(domain_hash(b"write-set-v1", canonical_write_set))
    }
}

impl PolicyDigest {
    /// Hash a canonical deterministic-policy encoding with a VSH domain separator.
    #[must_use]
    pub fn digest_canonical(canonical_policy: &[u8]) -> Self {
        Self::from_bytes(domain_hash(b"policy-v1", canonical_policy))
    }
}

impl RuntimeConfigDigest {
    /// Hash security-relevant runtime configuration with a VSH domain separator.
    #[must_use]
    pub fn digest_canonical(canonical_config: &[u8]) -> Self {
        Self::from_bytes(domain_hash(b"runtime-config-v1", canonical_config))
    }
}

impl IntentDigest {
    /// Hash transaction intent without retaining its potentially sensitive text.
    #[must_use]
    pub fn digest_text(intent: &str) -> Self {
        Self::from_bytes(domain_hash(b"intent-v1", intent.as_bytes()))
    }
}

impl PrincipalId {
    /// Hash a stable principal label without persisting it in transaction state.
    #[must_use]
    pub fn digest_label(label: &str) -> Self {
        Self::from_bytes(domain_hash(b"principal-v1", label.as_bytes()))
    }
}

impl HookId {
    /// Hash a stable hook label without retaining configuration text.
    #[must_use]
    pub fn digest_label(label: &str) -> Self {
        Self::from_bytes(domain_hash(b"hook-v1", label.as_bytes()))
    }
}

impl RequestEventId {
    /// Bind an event to one transaction, hook, and configured scope.
    #[must_use]
    pub fn derive(transaction: TransactionId, hook: HookId, scope_tag: u8) -> Self {
        let mut canonical = Vec::with_capacity(65);
        canonical.extend_from_slice(transaction.as_bytes());
        canonical.extend_from_slice(hook.as_bytes());
        canonical.push(scope_tag);
        Self::from_bytes(domain_hash(b"request-event-v1", &canonical))
    }
}

/// Exact fields covered by an independent approval grant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ApprovalBinding {
    /// Exact transaction artifact approved by the principal.
    pub transaction: TransactionId,
    /// Opaque identity of the independent principal.
    pub principal: PrincipalId,
    /// Host-supplied issuance time in Unix milliseconds.
    pub issued_at_unix_ms: u64,
    /// Exclusive expiry time in Unix milliseconds.
    pub expires_at_unix_ms: u64,
}

impl ApprovalBinding {
    /// Derive the immutable grant identity.
    #[must_use]
    pub fn approval_id(self) -> ApprovalId {
        let mut canonical = Vec::with_capacity(32 * 2 + 16);
        canonical.extend_from_slice(self.transaction.as_bytes());
        canonical.extend_from_slice(self.principal.as_bytes());
        canonical.extend_from_slice(&self.issued_at_unix_ms.to_le_bytes());
        canonical.extend_from_slice(&self.expires_at_unix_ms.to_le_bytes());
        ApprovalId::from_bytes(domain_hash(b"approval-v1", &canonical))
    }
}

/// Exact immutable inputs bound into an approval and commit identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransactionBinding {
    /// Immutable base snapshot observed by virtual execution.
    pub base_snapshot: SnapshotId,
    /// Exact canonical final diff.
    pub diff: DiffDigest,
    /// Exact dependencies read while deriving the result.
    pub read_set: ReadSetDigest,
    /// Exact preconditions for paths the transaction will write.
    pub write_set: WriteSetDigest,
    /// Exact untrusted program source.
    pub program: ProgramDigest,
    /// Exact deterministic policy configuration.
    pub policy: PolicyDigest,
    /// Security-relevant execution configuration and budgets.
    pub runtime_config: RuntimeConfigDigest,
    /// Optional out-of-band user intent shown to an approval principal.
    pub intent: Option<IntentDigest>,
}

impl TransactionBinding {
    /// Derive the single transaction identity to which approval and commit bind.
    #[must_use]
    pub fn transaction_id(self) -> TransactionId {
        let mut canonical = Vec::with_capacity(32 * 8 + 1);
        canonical.extend_from_slice(self.base_snapshot.as_bytes());
        canonical.extend_from_slice(self.diff.as_bytes());
        canonical.extend_from_slice(self.read_set.as_bytes());
        canonical.extend_from_slice(self.write_set.as_bytes());
        canonical.extend_from_slice(self.program.as_bytes());
        canonical.extend_from_slice(self.policy.as_bytes());
        canonical.extend_from_slice(self.runtime_config.as_bytes());
        match self.intent {
            Some(intent) => {
                canonical.push(1);
                canonical.extend_from_slice(intent.as_bytes());
            }
            None => canonical.push(0),
        }
        TransactionId::from_bytes(domain_hash(b"transaction-v1", &canonical))
    }
}

fn domain_hash(domain: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"vsh\0");
    hasher.update(&(domain.len() as u64).to_le_bytes());
    hasher.update(domain);
    hasher.update(&(payload.len() as u64).to_le_bytes());
    hasher.update(payload);
    *hasher.finalize().as_bytes()
}

fn encode_vpath(path: &VPath, output: &mut Vec<u8>) {
    let bytes = path.as_str().as_bytes();
    output.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    output.extend_from_slice(bytes);
}

/// The semantic kind of a virtual filesystem node.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum NodeKind {
    /// A regular file.
    File,
    /// A directory.
    Directory,
    /// An opaque symbolic link; the virtual filesystem never follows it implicitly.
    Symlink,
}

impl NodeKind {
    /// Return the stable tag used by canonical encodings.
    #[must_use]
    pub const fn canonical_tag(self) -> u8 {
        match self {
            Self::File => 1,
            Self::Directory => 2,
            Self::Symlink => 3,
        }
    }
}

/// Platform-specific identity of a host filesystem node.
///
/// Unix implementations encode device and inode; Windows implementations encode the
/// volume and file identity. Keeping two opaque words avoids host paths in receipts.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PlatformFileId {
    /// Platform-defined high word (for example, Unix device ID).
    pub high: u64,
    /// Platform-defined low word (for example, Unix inode ID).
    pub low: u64,
}

/// Metadata identity captured for one immutable base-snapshot node.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileStamp {
    /// Node kind observed without following a symbolic link.
    pub kind: NodeKind,
    /// Byte size reported by the host.
    pub size: u64,
    /// Portable permission/mode bits retained by VSH.
    pub mode: u32,
    /// Nanosecond modification time.
    pub mtime_ns: i128,
    /// Nanosecond metadata-change time when the host exposes it.
    pub ctime_ns: Option<i128>,
    /// Stable platform file identity for race detection.
    pub file_id: PlatformFileId,
}

/// Content identity carried by a node state.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum ContentVersion {
    /// Exact immutable content has been captured in the blob store.
    Blob(BlobId),
    /// Content is lazy; this metadata stamp must still match when it is captured.
    Stamp(FileStamp),
}

/// Canonical state of a virtual filesystem node.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeState {
    kind: NodeKind,
    size: u64,
    mode: u32,
    content: Option<ContentVersion>,
}

impl NodeState {
    /// Construct a synthetic directory state.
    #[must_use]
    pub const fn directory(mode: u32) -> Self {
        Self {
            kind: NodeKind::Directory,
            size: 0,
            mode,
            content: None,
        }
    }

    /// Construct a regular file backed by an immutable blob.
    #[must_use]
    pub const fn file(blob: BlobId, size: u64, mode: u32) -> Self {
        Self {
            kind: NodeKind::File,
            size,
            mode,
            content: Some(ContentVersion::Blob(blob)),
        }
    }

    /// Construct an opaque symbolic link backed by its target bytes.
    #[must_use]
    pub const fn symlink(blob: BlobId, size: u64, mode: u32) -> Self {
        Self {
            kind: NodeKind::Symlink,
            size,
            mode,
            content: Some(ContentVersion::Blob(blob)),
        }
    }

    /// Construct a lazily materialized state from verified host metadata.
    #[must_use]
    pub const fn from_stamp(stamp: FileStamp) -> Self {
        Self {
            kind: stamp.kind,
            size: stamp.size,
            mode: stamp.mode,
            content: Some(ContentVersion::Stamp(stamp)),
        }
    }

    /// Return the semantic node kind.
    #[must_use]
    pub const fn kind(self) -> NodeKind {
        self.kind
    }

    /// Return the node byte size.
    #[must_use]
    pub const fn size(self) -> u64 {
        self.size
    }

    /// Return retained permission/mode bits.
    #[must_use]
    pub const fn mode(self) -> u32 {
        self.mode
    }

    /// Return the exact or lazy content version, if applicable.
    #[must_use]
    pub const fn content(self) -> Option<ContentVersion> {
        self.content
    }

    /// Return a copy whose file/link content is now materialized.
    #[must_use]
    pub const fn with_blob(self, blob: BlobId, size: u64) -> Option<Self> {
        match self.kind {
            NodeKind::Directory => None,
            NodeKind::File | NodeKind::Symlink => Some(Self {
                kind: self.kind,
                size,
                mode: self.mode,
                content: Some(ContentVersion::Blob(blob)),
            }),
        }
    }

    /// Append this state to a stable length-delimited canonical encoding.
    pub fn encode_canonical(self, output: &mut Vec<u8>) {
        output.push(self.kind.canonical_tag());
        output.extend_from_slice(&self.size.to_le_bytes());
        output.extend_from_slice(&self.mode.to_le_bytes());
        match self.content {
            None => output.push(0),
            Some(ContentVersion::Blob(blob)) => {
                output.push(1);
                output.extend_from_slice(blob.as_bytes());
            }
            Some(ContentVersion::Stamp(stamp)) => {
                output.push(2);
                encode_stamp(stamp, output);
            }
        }
    }

    /// Return whether only metadata differs between two states.
    #[must_use]
    pub fn content_equivalent(self, other: Self) -> bool {
        self.kind == other.kind && self.size == other.size && self.content == other.content
    }
}

fn encode_stamp(stamp: FileStamp, output: &mut Vec<u8>) {
    output.push(stamp.kind.canonical_tag());
    output.extend_from_slice(&stamp.size.to_le_bytes());
    output.extend_from_slice(&stamp.mode.to_le_bytes());
    output.extend_from_slice(&stamp.mtime_ns.to_le_bytes());
    match stamp.ctime_ns {
        Some(value) => {
            output.push(1);
            output.extend_from_slice(&value.to_le_bytes());
        }
        None => output.push(0),
    }
    output.extend_from_slice(&stamp.file_id.high.to_le_bytes());
    output.extend_from_slice(&stamp.file_id.low.to_le_bytes());
}

/// Semantic category of one canonical diff entry.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DiffKind {
    /// A path absent from the base exists in final virtual state.
    Create,
    /// A base path is absent from final virtual state.
    Delete,
    /// Node kind or content changed.
    Modify,
    /// Only retained metadata changed.
    MetadataChange,
}

/// One path change in a canonical virtual filesystem diff.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiffEntry {
    /// Changed virtual path.
    pub path: VPath,
    /// Base state, or `None` when this is a creation.
    pub before: Option<NodeState>,
    /// Final virtual state, or `None` when this is a deletion.
    pub after: Option<NodeState>,
    /// Semantic change category derived from `before` and `after`.
    pub kind: DiffKind,
}

/// A persisted transaction state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TransactionState {
    /// The transaction record exists but execution has not started.
    Created,
    /// The untrusted program is executing against virtual state.
    Running,
    /// Virtual execution and canonical diff generation completed.
    VirtualComplete,
    /// Deterministic policy denied the transaction.
    Denied,
    /// Deterministic policy approved the transaction without a judge.
    AutoApproved,
    /// The transaction is awaiting an independent approval decision.
    PendingApproval,
    /// A configured commit hook rejected the exact transaction.
    Rejected,
    /// An independent principal approved the exact transaction.
    Approved,
    /// The transaction acquired the single-use commit reservation.
    Reserved,
    /// Recorded dependencies are being revalidated.
    Revalidating,
    /// The trusted committer is applying the canonical diff.
    Committing,
    /// The committed host state passed verification.
    Committed,
    /// Revalidation detected stale state before commit.
    Stale,
    /// Approval expired before reservation.
    Expired,
    /// Recovery is required after an interrupted commit.
    RecoveryRequired,
    /// The transaction failed without a successful commit.
    Failed,
}

impl TransactionState {
    /// Return whether `next` is a valid persisted state transition.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Created, Self::Running)
                | (Self::Running, Self::VirtualComplete | Self::Failed)
                | (
                    Self::VirtualComplete,
                    Self::Denied | Self::AutoApproved | Self::PendingApproval | Self::Failed
                )
                | (
                    Self::PendingApproval,
                    Self::Approved | Self::Denied | Self::Rejected | Self::Expired | Self::Failed
                )
                | (Self::AutoApproved, Self::PendingApproval | Self::Rejected)
                | (Self::AutoApproved | Self::Approved, Self::Reserved)
                | (Self::Approved, Self::Expired)
                | (Self::Reserved, Self::Revalidating | Self::Failed)
                | (
                    Self::Revalidating,
                    Self::Committing | Self::Stale | Self::Failed
                )
                | (Self::Committing, Self::Committed | Self::RecoveryRequired)
                | (Self::RecoveryRequired, Self::Committed | Self::Failed)
        )
    }

    /// Return whether no normal transition may leave this state.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Denied
                | Self::Rejected
                | Self::Committed
                | Self::Stale
                | Self::Expired
                | Self::Failed
        )
    }
}

/// An invalid persisted transaction transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransitionError {
    /// State before the rejected transition.
    pub from: TransactionState,
    /// Requested next state.
    pub to: TransactionState,
}

impl fmt::Display for TransitionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid transaction transition: {:?} -> {:?}",
            self.from, self.to
        )
    }
}

impl Error for TransitionError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_fast_path_matches_normalization_oracle() {
        fn oracle(input: &str) -> Result<VPath, VPathError> {
            if input.is_empty() {
                return Err(VPathError::Empty);
            }
            if input.contains('\0') {
                return Err(VPathError::NulByte);
            }
            let portable = input.replace('\\', "/");
            if portable.starts_with('/') {
                return Err(VPathError::Absolute);
            }
            if is_windows_prefix(portable.split('/').next().unwrap_or_default()) {
                return Err(VPathError::PlatformPrefix);
            }
            let mut components = Vec::new();
            for component in portable.split('/') {
                match component {
                    "" | "." => {}
                    ".." => {
                        if components.pop().is_none() {
                            return Err(VPathError::EscapesRoot);
                        }
                    }
                    value => components.push(value),
                }
            }
            Ok(VPath(if components.is_empty() {
                ".".to_owned()
            } else {
                components.join("/")
            }))
        }
        let components = ["", ".", "..", "a", "é", "C:", "x:y", "a\0b", "a\\b"];
        for first in components {
            assert_eq!(VPath::parse(first), oracle(first));
            for second in components {
                for third in components {
                    for separator in ["/", "\\"] {
                        let candidate = [first, second, third].join(separator);
                        assert_eq!(
                            VPath::parse(&candidate),
                            oracle(&candidate),
                            "{candidate:?}"
                        );
                        // Joining is defined by normalizing the combined path,
                        // not by parsing the child independently.
                        let combined = format!("parent/{candidate}");
                        assert_eq!(
                            VPath::parse("parent").unwrap().join(&candidate),
                            oracle(&combined),
                            "joined {candidate:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn vpath_normalizes_portable_components() {
        let path = VPath::parse("src\\vsh/./core/../lib.rs").unwrap();
        assert_eq!(path.as_str(), "src/vsh/lib.rs");
        assert!(!path.is_root());
        assert_eq!(path.to_string(), "src/vsh/lib.rs");
    }

    #[test]
    fn vpath_normalizes_root() {
        for candidate in [".", "./", "a/..", "a//../."] {
            let path = VPath::parse(candidate).unwrap();
            assert!(path.is_root(), "candidate: {candidate}");
            assert_eq!(path.as_str(), ".");
        }
    }

    #[test]
    fn vpath_rejects_ambiguous_or_escaping_paths() {
        let cases = [
            ("", VPathError::Empty),
            ("/etc/passwd", VPathError::Absolute),
            ("\\\\server\\share", VPathError::Absolute),
            ("C:\\Windows", VPathError::PlatformPrefix),
            ("../secret", VPathError::EscapesRoot),
            ("a/../../secret", VPathError::EscapesRoot),
            ("a\0b", VPathError::NulByte),
        ];

        for (candidate, expected) in cases {
            assert_eq!(
                VPath::parse(candidate),
                Err(expected),
                "candidate: {candidate}"
            );
        }
    }

    #[test]
    fn vpath_parent_join_and_rebase_preserve_root_contract() {
        let root = VPath::parse(".").unwrap();
        let source = VPath::parse("src/tree/file.txt").unwrap();
        let subtree = VPath::parse("src/tree").unwrap();
        let destination = VPath::parse("lib").unwrap();

        assert_eq!(source.parent().unwrap().as_str(), "src/tree");
        assert_eq!(root.parent(), None);
        assert_eq!(root.join("a/b").unwrap().as_str(), "a/b");
        assert_eq!(subtree.join("child").unwrap().as_str(), "src/tree/child");
        assert!(source.is_within(&root));
        assert!(source.is_within(&subtree));
        assert!(!subtree.is_within(&source));
        assert_eq!(source.relative_to(&subtree), Some("file.txt"));
        assert_eq!(
            source.rebase(&subtree, &destination).unwrap().unwrap(),
            VPath::parse("lib/file.txt").unwrap()
        );
        assert_eq!(source.rebase(&destination, &subtree).unwrap(), None);
        assert_eq!(root.relative_to(&root), Some(""));
        assert_eq!(source.relative_to(&root), Some("src/tree/file.txt"));
        assert_eq!(VPath::try_from("src/tree/file.txt").unwrap(), source);
    }

    #[test]
    fn public_value_errors_and_optional_stamp_encoding_are_stable() {
        let path_messages = [
            VPathError::Empty,
            VPathError::Absolute,
            VPathError::EscapesRoot,
            VPathError::NulByte,
            VPathError::PlatformPrefix,
        ]
        .map(|error| error.to_string());
        assert!(path_messages.iter().all(|message| !message.is_empty()));

        assert_eq!(
            ParseDigestError::InvalidLength { observed: 1 }.to_string(),
            "digest must be 64 hexadecimal bytes, got 1"
        );
        assert_eq!(
            ParseDigestError::InvalidHex { index: 2 }.to_string(),
            "digest contains non-hexadecimal byte at index 2"
        );

        let stamp = FileStamp {
            kind: NodeKind::File,
            size: 0,
            mode: 0o644,
            mtime_ns: 0,
            ctime_ns: None,
            file_id: PlatformFileId { high: 0, low: 0 },
        };
        let mut encoded = Vec::new();
        NodeState::from_stamp(stamp).encode_canonical(&mut encoded);
        assert!(!encoded.is_empty());
    }

    #[test]
    fn digest_ids_are_fixed_width_lower_hex() {
        let id = BlobId::from_bytes([0xab; 32]);
        assert_eq!(id.as_bytes(), &[0xab; 32]);
        assert_eq!(id.to_string(), "ab".repeat(32));
        assert_eq!(format!("{id:?}"), id.to_string());

        let snapshot = SnapshotId::from_bytes([1; 32]);
        let transaction = TransactionId::from_bytes([2; 32]);
        assert_ne!(snapshot.to_string(), transaction.to_string());
    }

    #[test]
    fn digest_ids_parse_exact_hex_without_a_dependency() {
        let expected = TransactionId::from_bytes([0xab; 32]);
        assert_eq!("ab".repeat(32).parse::<TransactionId>().unwrap(), expected);
        assert_eq!("AB".repeat(32).parse::<TransactionId>().unwrap(), expected);
        assert_eq!(
            "ab".parse::<TransactionId>(),
            Err(ParseDigestError::InvalidLength { observed: 2 })
        );
        let mut invalid = "ab".repeat(32);
        invalid.replace_range(7..8, "z");
        assert_eq!(
            invalid.parse::<TransactionId>(),
            Err(ParseDigestError::InvalidHex { index: 7 })
        );
    }

    #[test]
    fn content_and_domain_hashes_are_deterministic_and_separated() {
        let payload = b"canonical bytes";
        assert_eq!(BlobId::digest(payload), BlobId::digest(payload));
        assert_ne!(
            SnapshotId::digest_manifest(payload).as_bytes(),
            DiffDigest::digest_canonical(payload).as_bytes()
        );
        assert_ne!(
            DiffDigest::digest_canonical(payload).as_bytes(),
            DirectoryDigest::digest_canonical(payload).as_bytes()
        );
    }

    #[test]
    fn node_state_canonical_encoding_captures_metadata_and_content() {
        let blob = BlobId::digest(b"data");
        let file = NodeState::file(blob, 4, 0o644);
        let changed_mode = NodeState::file(blob, 4, 0o600);
        let changed_content = NodeState::file(BlobId::digest(b"else"), 4, 0o644);
        let mut encoded = Vec::new();
        file.encode_canonical(&mut encoded);

        assert!(!encoded.is_empty());
        assert!(file.content_equivalent(changed_mode));
        assert!(!file.content_equivalent(changed_content));
        assert_eq!(file.kind(), NodeKind::File);
        assert_eq!(file.size(), 4);
        assert_eq!(file.mode(), 0o644);
        assert_eq!(file.content(), Some(ContentVersion::Blob(blob)));
        assert!(NodeState::directory(0o755).with_blob(blob, 4).is_none());
    }

    #[test]
    fn transaction_happy_path_is_valid() {
        let states = [
            TransactionState::Created,
            TransactionState::Running,
            TransactionState::VirtualComplete,
            TransactionState::AutoApproved,
            TransactionState::Reserved,
            TransactionState::Revalidating,
            TransactionState::Committing,
            TransactionState::Committed,
        ];
        for pair in states.windows(2) {
            assert!(pair[0].can_transition_to(pair[1]), "pair: {pair:?}");
        }
        assert!(TransactionState::Committed.is_terminal());
    }

    #[test]
    fn transaction_rejects_replay_and_skipped_states() {
        assert!(!TransactionState::Approved.can_transition_to(TransactionState::Committed));
        assert!(!TransactionState::Committed.can_transition_to(TransactionState::Reserved));
        assert!(!TransactionState::Denied.can_transition_to(TransactionState::Approved));
        assert!(TransactionState::Denied.is_terminal());
        assert!(!TransactionState::RecoveryRequired.is_terminal());
    }
}
