use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use vsh_policy::{read_set_digest, write_set_digest};
use vsh_store::{
    BlobStore, BlobStoreError, DataDirectory, MemoryTransactionStore, TransactionRecord,
    TransactionStore, TransactionStoreError,
};
use vsh_types::{
    BlobId, DiffEntry, DiffKind, FileStamp, NodeKind, NodeState, PlatformFileId, PolicyDigest,
    ProgramDigest, RuntimeConfigDigest, TransactionBinding, TransactionId, TransactionState, VPath,
};
use vsh_vfs::{CanonicalDiff, ReadObservation, SnapshotError, VirtualFs, WritePrecondition};

use super::*;

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(name: &str) -> Self {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "vsh-commit-test-{}-{sequence}-{name}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn workspace(&self) -> PathBuf {
        self.0.join("workspace")
    }

    fn data(&self) -> PathBuf {
        self.0.join("data")
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn path(value: &str) -> VPath {
    VPath::parse(value).unwrap()
}

fn fixture(name: &str) -> (TestDirectory, Committer) {
    let directory = TestDirectory::new(name);
    fs::create_dir_all(directory.workspace()).unwrap();
    fs::write(directory.workspace().join("old.txt"), b"old").unwrap();
    let blobs = BlobStore::open(directory.data()).unwrap();
    let committer = Committer::open(directory.workspace(), blobs, CommitConfig::default()).unwrap();
    (directory, committer)
}

fn binding(vfs: &VirtualFs, diff: &CanonicalDiff) -> TransactionBinding {
    TransactionBinding {
        base_snapshot: vfs.base_snapshot_id(),
        diff: diff.digest(),
        read_set: read_set_digest(vfs.read_set()),
        write_set: write_set_digest(vfs.write_set()),
        program: ProgramDigest::digest_source("test-program"),
        policy: PolicyDigest::digest_canonical(b"test-policy"),
        runtime_config: RuntimeConfigDigest::digest_canonical(b"test-runtime"),
        intent: None,
    }
}

fn reserve(
    store: &MemoryTransactionStore,
    binding: &TransactionBinding,
) -> vsh_store::CommitReservation {
    let id = binding.transaction_id();
    store
        .create(TransactionRecord::new(id, binding.base_snapshot))
        .unwrap();
    store
        .compare_and_transition(id, TransactionState::Created, TransactionState::Running)
        .unwrap();
    store
        .compare_and_transition(
            id,
            TransactionState::Running,
            TransactionState::VirtualComplete,
        )
        .unwrap();
    store
        .compare_and_transition(
            id,
            TransactionState::VirtualComplete,
            TransactionState::AutoApproved,
        )
        .unwrap();
    store.reserve(id, 0).unwrap()
}

fn build_fault_transaction(
    committer: &Committer,
) -> (VirtualFs, CanonicalDiff, TransactionBinding) {
    let snapshot = committer.snapshot(SnapshotLimits::default()).unwrap();
    let mut vfs = VirtualFs::new(snapshot);
    vfs.write(&path("old.txt"), b"new").unwrap();
    vfs.mkdir(&path("created"), 0o755).unwrap();
    vfs.write(&path("created/new.txt"), b"created").unwrap();
    let diff = vfs.canonical_diff().unwrap();
    let binding = binding(&vfs, &diff);
    (vfs, diff, binding)
}

fn workspace_is_original(root: &Path) -> bool {
    fs::read(root.join("old.txt")).ok().as_deref() == Some(b"old") && !root.join("created").exists()
}

fn workspace_is_committed(root: &Path) -> bool {
    fs::read(root.join("old.txt")).ok().as_deref() == Some(b"new")
        && fs::read(root.join("created/new.txt")).ok().as_deref() == Some(b"created")
}

#[test]
fn snapshot_is_lazy_and_hides_the_trusted_runtime_directory() {
    let (_directory, committer) = fixture("snapshot");
    let snapshot = committer.snapshot(SnapshotLimits::default()).unwrap();
    assert_eq!(snapshot.metrics().lazy_content_nodes, 1);
    let mut vfs = VirtualFs::new(snapshot);
    assert!(!vfs.exists(&path(".vsh-runtime")));
    assert_eq!(vfs.read(&path("old.txt")).unwrap(), b"old");
}

#[test]
fn typed_commit_error_contract_preserves_messages_and_sources() {
    let directory = TestDirectory::new("error-contract");
    let invalid_data_path = directory.0.join("not-a-directory");
    fs::write(&invalid_data_path, b"file").unwrap();
    let data_error = DataDirectory::open_trusted(&invalid_data_path).unwrap_err();
    let transaction = TransactionId::from_bytes([1; 32]);
    let other_transaction = TransactionId::from_bytes([2; 32]);
    let node = NodeState::file(BlobId::digest(b"x"), 1, 0o644);
    let io_error = || io::Error::other("expected test failure");
    let sourced = [
        CommitError::Plan(CommitPlanError::RootMutation),
        CommitError::Host(HostError::Io {
            operation: "test",
            path: path("source.txt"),
            source: io_error(),
        }),
        CommitError::Store(TransactionStoreError::NotFound { id: transaction }),
        CommitError::Blob(BlobStoreError::Io {
            operation: "test",
            path: PathBuf::from("blob"),
            source: io_error(),
        }),
        CommitError::DataDirectory(data_error),
        CommitError::Journal(JournalError::Io(io_error())),
        CommitError::PlanDecode(PlanDecodeError::Tag),
        CommitError::InternalIo {
            operation: "test",
            source: io_error(),
        },
    ];
    for error in sourced {
        assert!(!error.to_string().is_empty());
        assert!(std::error::Error::source(&error).is_some());
    }

    let unsourced = [
        CommitError::Binding {
            reserved_transaction: transaction,
            plan_transaction: other_transaction,
        },
        CommitError::BaseSnapshotBinding,
        CommitError::DependencyLimit {
            observed: 2,
            maximum: 1,
        },
        CommitError::PlanSize {
            observed: 2,
            maximum: 1,
        },
        CommitError::TransactionWorkspaceExists { transaction },
        CommitError::UnsafeBlobStore {
            workspace_root: PathBuf::from("workspace"),
            blobs_directory: PathBuf::from("workspace/blobs"),
        },
        CommitError::Stale {
            conflicts: vec![RevalidationConflict::Metadata {
                path: path("source.txt"),
                expected: Some(node),
                actual: None,
            }],
        },
        CommitError::Verification(Box::new(VerificationFailure {
            path: path("source.txt"),
            expected: Some(node),
            actual: None,
        })),
        CommitError::FaultInjected {
            point: FaultPoint::PlanSynced,
        },
        CommitError::RecoveryRequired {
            transaction,
            cause: "test".to_owned(),
        },
        CommitError::RecoveryConflict(RecoveryConflict {
            transaction,
            path: None,
            reason: "test",
        }),
        CommitError::InvalidRecoveryState {
            transaction,
            state: TransactionState::Created,
        },
    ];
    for error in unsourced {
        assert!(!error.to_string().is_empty());
        assert!(std::error::Error::source(&error).is_none());
    }
}

#[test]
fn nested_commit_error_types_have_stable_distinct_messages() {
    let plan_errors = [
        CommitPlanError::DiffDigestMismatch,
        CommitPlanError::ReadSetDigestMismatch,
        CommitPlanError::WriteSetDigestMismatch,
        CommitPlanError::RootMutation,
        CommitPlanError::ReservedPath {
            path: path(".vsh-runtime/data"),
        },
        CommitPlanError::MissingWritePrecondition { path: path("file") },
        CommitPlanError::MissingParentDependency {
            path: path("dir/file"),
            parent: path("dir"),
        },
        CommitPlanError::BeforeStateMismatch { path: path("file") },
        CommitPlanError::UnmaterializedAfterState { path: path("file") },
        CommitPlanError::TooManyOperations {
            observed: 2,
            maximum: 1,
        },
        CommitPlanError::PathTooLong {
            path: path("file"),
            maximum: 1,
        },
        CommitPlanError::OperationCountOverflow,
    ];
    let plan_messages = plan_errors.map(|error| error.to_string());
    assert!(plan_messages.iter().all(|message| !message.is_empty()));
    assert_eq!(
        plan_messages
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        plan_messages.len()
    );

    let decode_errors = [
        PlanDecodeError::Truncated,
        PlanDecodeError::Checksum,
        PlanDecodeError::Magic,
        PlanDecodeError::Tag,
        PlanDecodeError::Utf8,
        PlanDecodeError::Path,
        PlanDecodeError::State,
        PlanDecodeError::Limit,
        PlanDecodeError::TrailingBytes,
    ];
    assert_eq!(
        decode_errors
            .map(|error| error.to_string())
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        decode_errors.len()
    );
}

#[test]
fn journal_and_host_error_types_have_stable_sources() {
    let journal_errors = [
        JournalError::Magic,
        JournalError::RecordLength,
        JournalError::Checksum,
        JournalError::Sequence,
        JournalError::Tag,
        JournalError::Marker,
    ];
    for error in journal_errors {
        assert!(!error.to_string().is_empty());
        assert!(std::error::Error::source(&error).is_none());
    }
    let journal_io = JournalError::Io(io::Error::other("test"));
    assert!(std::error::Error::source(&journal_io).is_some());

    let stamp = FileStamp {
        kind: NodeKind::File,
        size: 1,
        mode: 0o644,
        mtime_ns: 1,
        ctime_ns: Some(2),
        file_id: PlatformFileId { high: 3, low: 4 },
    };
    let host_errors = [
        HostError::io("read", &path("file"), io::Error::other("test")),
        HostError::InternalIo {
            operation: "read",
            path: PathBuf::from("internal"),
            source: io::Error::other("test"),
        },
        HostError::UnsupportedNode { path: path("file") },
        HostError::NonUtf8Name {
            parent: VPath::root(),
            name: OsString::from("name"),
        },
        HostError::NonUtf8Symlink { path: path("link") },
        HostError::MissingFileIdentity { path: path("file") },
        HostError::Unstable {
            path: path("file"),
            before: Box::new(stamp),
            after: Box::new(FileStamp { size: 2, ..stamp }),
        },
        HostError::SnapshotLimit {
            limit: "nodes",
            observed: 2,
            maximum: 1,
        },
        HostError::Snapshot(SnapshotError::DuplicatePath { path: path("file") }),
    ];
    for error in host_errors {
        assert!(!error.to_string().is_empty());
        assert_eq!(
            std::error::Error::source(&error).is_some(),
            matches!(
                error,
                HostError::Io { .. } | HostError::InternalIo { .. } | HostError::Snapshot(_)
            )
        );
    }
}

#[test]
fn snapshot_limits_fail_closed_at_each_independent_bound() {
    let directory = TestDirectory::new("snapshot-limits");
    let workspace = directory.workspace();
    fs::create_dir_all(workspace.join("deep")).unwrap();
    fs::write(workspace.join("deep/file.txt"), b"four").unwrap();
    let blobs = BlobStore::open(directory.data()).unwrap();
    let committer = Committer::open(&workspace, blobs, CommitConfig::default()).unwrap();

    let cases = [
        (
            SnapshotLimits {
                max_nodes: 1,
                ..SnapshotLimits::default()
            },
            "node-count",
        ),
        (
            SnapshotLimits {
                max_depth: 0,
                ..SnapshotLimits::default()
            },
            "depth",
        ),
        (
            SnapshotLimits {
                max_total_file_bytes: 1,
                ..SnapshotLimits::default()
            },
            "total-file-bytes",
        ),
    ];
    for (limits, expected_limit) in cases {
        assert!(matches!(
            committer.snapshot(limits),
            Err(CommitError::Host(HostError::SnapshotLimit { limit, .. }))
                if limit == expected_limit
        ));
    }
}

#[test]
fn directory_revalidation_is_bounded_after_external_growth() {
    let directory = TestDirectory::new("directory-revalidation-limit");
    let workspace = directory.workspace();
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("first.txt"), b"first").unwrap();
    let blobs = BlobStore::open(directory.data()).unwrap();
    let committer = Committer::open(
        &workspace,
        blobs,
        CommitConfig {
            max_dependencies: 1,
            ..CommitConfig::default()
        },
    )
    .unwrap();
    let snapshot = committer.snapshot(SnapshotLimits::default()).unwrap();
    let mut vfs = VirtualFs::new(snapshot);
    vfs.read_dir(&VPath::root()).unwrap();
    let diff = vfs.canonical_diff().unwrap();
    let mut read_set = vfs.read_set().clone();
    read_set.get_mut(&VPath::root()).unwrap().metadata = None;
    let mut binding = binding(&vfs, &diff);
    binding.read_set = read_set_digest(&read_set);
    let plan = CommitPlan::new(&binding, &diff, &read_set, vfs.write_set()).unwrap();

    fs::write(workspace.join("second.txt"), b"second").unwrap();

    assert!(matches!(
        committer.revalidate(&plan),
        Err(CommitError::Host(HostError::SnapshotLimit {
            limit: "directory-entries",
            observed: 2,
            maximum: 1,
        }))
    ));
}

#[cfg(unix)]
#[test]
fn snapshot_rejects_nonportable_names_and_special_nodes() {
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::net::UnixListener;

    let names = TestDirectory::new("non-utf8-name");
    fs::create_dir(names.workspace()).unwrap();
    let non_utf8 = fs::write(
        names.workspace().join(OsString::from_vec(vec![0xff])),
        b"opaque",
    );
    match non_utf8 {
        Ok(()) => {
            let blobs = BlobStore::open(names.data()).unwrap();
            let committer =
                Committer::open(names.workspace(), blobs, CommitConfig::default()).unwrap();
            assert!(matches!(
                committer.snapshot(SnapshotLimits::default()),
                Err(CommitError::Host(HostError::NonUtf8Name { .. }))
            ));
        }
        Err(source) if source.kind() == io::ErrorKind::PermissionDenied => {}
        Err(source) => panic!("unexpected non-UTF-8 fixture failure: {source}"),
    }

    let nodes = TestDirectory::new("special-node");
    fs::create_dir(nodes.workspace()).unwrap();
    let _listener = match UnixListener::bind(nodes.workspace().join("socket")) {
        Ok(listener) => listener,
        Err(source) if source.kind() == io::ErrorKind::PermissionDenied => return,
        Err(source) => panic!("unexpected Unix-socket fixture failure: {source}"),
    };
    let blobs = BlobStore::open(nodes.data()).unwrap();
    let committer = Committer::open(nodes.workspace(), blobs, CommitConfig::default()).unwrap();
    assert!(matches!(
        committer.snapshot(SnapshotLimits::default()),
        Err(CommitError::Host(HostError::UnsupportedNode { .. }))
    ));
}

#[test]
fn symlink_target_validation_never_allows_a_virtual_root_escape() {
    let link = path("dir/link");
    assert!(matches!(
        host::validate_symlink_target(&link, b""),
        Err(HostError::Io { .. })
    ));
    assert!(matches!(
        host::validate_symlink_target(&link, b"../../../outside"),
        Err(HostError::Io { .. })
    ));
    assert!(matches!(
        host::validate_symlink_target(&link, &[0xff]),
        Err(HostError::NonUtf8Symlink { .. })
    ));
    assert_eq!(
        host::validate_symlink_target(&link, b"../target").unwrap(),
        PathBuf::from("../target")
    );
}

#[cfg(unix)]
#[test]
fn relocated_runtime_directory_is_never_exposed_as_workspace_data() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new("relocated-runtime");
    fs::create_dir_all(directory.workspace()).unwrap();
    let outside = directory.0.join("outside");
    fs::create_dir(&outside).unwrap();
    let (committer, data) =
        Committer::open_with_workspace_data(directory.workspace(), CommitConfig::default())
            .unwrap();
    let canonical_workspace = fs::canonicalize(directory.workspace()).unwrap();
    committer.artifact_store().put(b"pinned").unwrap();

    let runtime = directory.workspace().join(".vsh-runtime");
    let relocated = directory.workspace().join("runtime-relocated");
    fs::rename(&runtime, &relocated).unwrap();
    symlink(&outside, &runtime).unwrap();

    assert!(matches!(
        committer.snapshot(SnapshotLimits::default()),
        Err(CommitError::InternalIo { .. })
    ));
    assert_eq!(
        data.path(),
        canonical_workspace.join(".vsh-runtime/data").as_path()
    );
    assert!(relocated.join("data/blobs").is_dir());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
}

#[test]
fn relocated_workspace_is_rejected_before_further_observation() {
    let directory = TestDirectory::new("relocated-workspace");
    let workspace = directory.workspace();
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("visible.txt"), b"original").unwrap();
    let (committer, _data) =
        Committer::open_with_workspace_data(&workspace, CommitConfig::default()).unwrap();
    let relocated = directory.0.join("workspace-relocated");
    fs::rename(&workspace, &relocated).unwrap();
    fs::create_dir(&workspace).unwrap();

    assert!(matches!(
        committer.snapshot(SnapshotLimits::default()),
        Err(CommitError::InternalIo { .. })
    ));
    assert_eq!(fs::read_dir(&workspace).unwrap().count(), 0);
    assert_eq!(
        fs::read(relocated.join("visible.txt")).unwrap(),
        b"original"
    );
    assert!(relocated.join(".vsh-runtime/data/blobs").is_dir());
}

#[test]
fn caller_blob_store_cannot_overlap_the_workspace() {
    let directory = TestDirectory::new("overlapping-blob-store");
    let workspace = directory.workspace();
    fs::create_dir(&workspace).unwrap();
    let blobs = BlobStore::open(workspace.join("visible-data")).unwrap();

    let result = Committer::open(&workspace, blobs, CommitConfig::default());

    assert!(matches!(result, Err(CommitError::UnsafeBlobStore { .. })));
    assert!(!workspace.join(".vsh-runtime").exists());
}

#[cfg(unix)]
#[test]
fn snapshot_entry_metadata_does_not_follow_symlinks() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new("snapshot-symlink");
    fs::create_dir_all(directory.workspace()).unwrap();
    symlink("/etc/passwd", directory.workspace().join("escape")).unwrap();
    let blobs = BlobStore::open(directory.data()).unwrap();
    let committer = Committer::open(directory.workspace(), blobs, CommitConfig::default()).unwrap();

    let snapshot = committer.snapshot(SnapshotLimits::default()).unwrap();
    let mut vfs = VirtualFs::new(snapshot);

    assert_eq!(
        vfs.metadata(&path("escape")).unwrap().kind(),
        NodeKind::Symlink
    );
    assert_eq!(vfs.read_link(&path("escape")).unwrap(), b"/etc/passwd");
}

#[test]
fn commit_revalidates_applies_and_verifies_the_exact_plan() {
    let (directory, committer) = fixture("success");
    let (vfs, diff, binding) = build_fault_transaction(&committer);
    let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
    let store = MemoryTransactionStore::default();
    let reservation = reserve(&store, &binding);
    let receipt = committer.commit(&store, reservation, &plan).unwrap();

    assert_eq!(receipt.transaction, binding.transaction_id());
    assert!(!receipt.cleanup_pending);
    assert!(workspace_is_committed(&directory.workspace()));
    assert_eq!(
        store.get(binding.transaction_id()).unwrap().state(),
        TransactionState::Committed
    );
}

#[test]
fn bounded_preflight_failure_finalizes_the_consumed_reservation() {
    let directory = TestDirectory::new("bounded-preflight");
    fs::create_dir_all(directory.workspace()).unwrap();
    fs::write(directory.workspace().join("old.txt"), b"old").unwrap();
    let blobs = BlobStore::open(directory.data()).unwrap();
    let committer = Committer::open(
        directory.workspace(),
        blobs,
        CommitConfig {
            max_operations: 0,
            ..CommitConfig::default()
        },
    )
    .unwrap();
    let (vfs, diff, binding) = build_fault_transaction(&committer);
    let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
    let store = MemoryTransactionStore::default();
    let reservation = reserve(&store, &binding);

    let error = committer.commit(&store, reservation, &plan).unwrap_err();

    assert!(matches!(
        error,
        CommitError::Plan(CommitPlanError::TooManyOperations { maximum: 0, .. })
    ));
    assert_eq!(
        store.get(binding.transaction_id()).unwrap().state(),
        TransactionState::Failed
    );
    assert!(workspace_is_original(&directory.workspace()));
    assert_eq!(
        fs::read_dir(directory.workspace().join(".vsh-runtime/transactions"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn workspace_identity_failure_finalizes_the_consumed_reservation() {
    let (directory, committer) = fixture("workspace-preflight");
    let (vfs, diff, binding) = build_fault_transaction(&committer);
    let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
    let store = MemoryTransactionStore::default();
    let reservation = reserve(&store, &binding);
    let workspace = directory.workspace();
    let relocated = directory.0.join("workspace-before-commit");
    fs::rename(&workspace, &relocated).unwrap();
    fs::create_dir(&workspace).unwrap();

    let error = committer.commit(&store, reservation, &plan).unwrap_err();

    assert!(matches!(error, CommitError::InternalIo { .. }));
    assert_eq!(
        store.get(binding.transaction_id()).unwrap().state(),
        TransactionState::Failed
    );
    assert_eq!(fs::read_dir(&workspace).unwrap().count(), 0);
    assert!(workspace_is_original(&relocated));
}

#[cfg(unix)]
#[test]
fn commit_installs_opaque_symlinks_and_quarantines_subtrees() {
    use std::os::unix::fs::symlink;

    let directory = TestDirectory::new("symlink-subtree-commit");
    let workspace = directory.workspace();
    fs::create_dir_all(workspace.join("tree/nested")).unwrap();
    fs::write(workspace.join("target.txt"), b"target").unwrap();
    fs::write(workspace.join("tree/nested/delete.txt"), b"delete").unwrap();
    symlink("target.txt", workspace.join("old-link")).unwrap();
    let blobs = BlobStore::open(directory.data()).unwrap();
    let committer = Committer::open(&workspace, blobs, CommitConfig::default()).unwrap();
    let snapshot = committer.snapshot(SnapshotLimits::default()).unwrap();
    let mut vfs = VirtualFs::new(snapshot);
    vfs.rename(&path("old-link"), &path("new-link")).unwrap();
    vfs.remove_tree(&path("tree")).unwrap();
    let diff = vfs.canonical_diff().unwrap();
    let binding = binding(&vfs, &diff);
    let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
    let store = MemoryTransactionStore::default();

    let receipt = committer
        .commit(&store, reserve(&store, &binding), &plan)
        .unwrap();

    assert!(receipt.operations >= 3);
    assert_eq!(
        fs::read_link(workspace.join("new-link")).unwrap(),
        Path::new("target.txt")
    );
    assert!(!workspace.join("old-link").exists());
    assert!(!workspace.join("tree").exists());
    assert_eq!(fs::read(workspace.join("target.txt")).unwrap(), b"target");
}

#[cfg(unix)]
#[test]
fn commit_applies_and_verifies_directory_mode_changes() {
    use std::os::unix::fs::PermissionsExt;

    let directory = TestDirectory::new("directory-mode-commit");
    let workspace = directory.workspace();
    fs::create_dir_all(workspace.join("mode-dir")).unwrap();
    fs::set_permissions(
        workspace.join("mode-dir"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let blobs = BlobStore::open(directory.data()).unwrap();
    let committer = Committer::open(&workspace, blobs, CommitConfig::default()).unwrap();
    let snapshot = committer.snapshot(SnapshotLimits::default()).unwrap();
    let base_snapshot = snapshot.id();
    let mut vfs = VirtualFs::new(snapshot);
    let root = VPath::root();
    let root_state = vfs.metadata(&root).unwrap();
    let mode_path = path("mode-dir");
    let before = vfs.metadata(&mode_path).unwrap();
    let after = NodeState::directory(0o700);
    let diff = CanonicalDiff::from_entries(vec![DiffEntry {
        path: mode_path.clone(),
        kind: DiffKind::Modify,
        before: Some(before),
        after: Some(after),
    }])
    .unwrap();
    let read_set = BTreeMap::from([(
        root,
        ReadObservation {
            metadata: Some(Some(root_state)),
            ..ReadObservation::default()
        },
    )]);
    let write_set = BTreeMap::from([(
        mode_path,
        WritePrecondition {
            expected: Some(before),
        },
    )]);
    let binding = TransactionBinding {
        base_snapshot,
        diff: diff.digest(),
        read_set: read_set_digest(&read_set),
        write_set: write_set_digest(&write_set),
        program: ProgramDigest::digest_source("chmod-test"),
        policy: PolicyDigest::digest_canonical(b"test-policy"),
        runtime_config: RuntimeConfigDigest::digest_canonical(b"test-runtime"),
        intent: None,
    };
    let plan = CommitPlan::new(&binding, &diff, &read_set, &write_set).unwrap();
    let store = MemoryTransactionStore::default();

    committer
        .commit(&store, reserve(&store, &binding), &plan)
        .unwrap();

    assert_eq!(
        fs::metadata(workspace.join("mode-dir"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o700
    );
}

#[test]
fn stale_write_precondition_never_overwrites_external_work() {
    let (directory, committer) = fixture("stale");
    let snapshot = committer.snapshot(SnapshotLimits::default()).unwrap();
    let mut vfs = VirtualFs::new(snapshot);
    vfs.write(&path("old.txt"), b"transaction").unwrap();
    let diff = vfs.canonical_diff().unwrap();
    let binding = binding(&vfs, &diff);
    let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
    let store = MemoryTransactionStore::default();
    let reservation = reserve(&store, &binding);

    fs::write(directory.workspace().join("old.txt"), b"external").unwrap();
    let error = committer.commit(&store, reservation, &plan).unwrap_err();
    assert!(matches!(error, CommitError::Stale { .. }));
    assert_eq!(
        fs::read(directory.workspace().join("old.txt")).unwrap(),
        b"external"
    );
    assert_eq!(
        store.get(binding.transaction_id()).unwrap().state(),
        TransactionState::Stale
    );
}

#[test]
fn overlapping_committers_serialize_revalidation_and_only_one_writer_wins() {
    let directory = TestDirectory::new("overlapping-committers");
    let workspace = directory.workspace();
    fs::create_dir_all(&workspace).unwrap();
    fs::write(workspace.join("shared.txt"), b"base").unwrap();
    let blobs = BlobStore::open(directory.data()).unwrap();
    let first_committer =
        Committer::open(&workspace, blobs.clone(), CommitConfig::default()).unwrap();
    let second_committer = Committer::open(&workspace, blobs, CommitConfig::default()).unwrap();

    let mut first_vfs =
        VirtualFs::new(first_committer.snapshot(SnapshotLimits::default()).unwrap());
    first_vfs.write(&path("shared.txt"), b"first").unwrap();
    let first_diff = first_vfs.canonical_diff().unwrap();
    let first_binding = binding(&first_vfs, &first_diff);

    let mut second_vfs = VirtualFs::new(
        second_committer
            .snapshot(SnapshotLimits::default())
            .unwrap(),
    );
    second_vfs.write(&path("shared.txt"), b"second").unwrap();
    let second_diff = second_vfs.canonical_diff().unwrap();
    let second_binding = binding(&second_vfs, &second_diff);

    let store = Arc::new(MemoryTransactionStore::default());
    let first_reservation = reserve(&store, &first_binding);
    let second_reservation = reserve(&store, &second_binding);
    let release_first = Arc::new((Mutex::new(false), Condvar::new()));
    let (first_revalidated_tx, first_revalidated_rx) = mpsc::sync_channel(1);
    let first_release = Arc::clone(&release_first);
    let first_store = Arc::clone(&store);
    let first = thread::spawn(move || {
        let plan = CommitPlan::new(
            &first_binding,
            &first_diff,
            first_vfs.read_set(),
            first_vfs.write_set(),
        )
        .unwrap();
        first_committer.commit_with_faults(
            first_store.as_ref(),
            first_reservation,
            &plan,
            &move |point| {
                if point == FaultPoint::Revalidated {
                    first_revalidated_tx.send(()).unwrap();
                    let (mutex, condition) = &*first_release;
                    let mut released = mutex.lock().unwrap();
                    while !*released {
                        released = condition.wait(released).unwrap();
                    }
                }
                false
            },
        )
    });
    first_revalidated_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("first commit should pause after revalidation");

    let (second_started_tx, second_started_rx) = mpsc::sync_channel(1);
    let (second_revalidated_tx, second_revalidated_rx) = mpsc::sync_channel(1);
    let second_store = Arc::clone(&store);
    let second = thread::spawn(move || {
        second_started_tx.send(()).unwrap();
        let plan = CommitPlan::new(
            &second_binding,
            &second_diff,
            second_vfs.read_set(),
            second_vfs.write_set(),
        )
        .unwrap();
        second_committer.commit_with_faults(
            second_store.as_ref(),
            second_reservation,
            &plan,
            &move |point| {
                if point == FaultPoint::Revalidated {
                    let _ = second_revalidated_tx.try_send(());
                }
                false
            },
        )
    });
    second_started_rx.recv().unwrap();
    assert!(
        second_revalidated_rx
            .recv_timeout(Duration::from_millis(250))
            .is_err(),
        "a competing commit passed revalidation while the workspace lock was held"
    );

    let (mutex, condition) = &*release_first;
    *mutex.lock().unwrap() = true;
    condition.notify_all();
    first.join().unwrap().unwrap();
    let second_error = second.join().unwrap().unwrap_err();

    assert!(matches!(second_error, CommitError::Stale { .. }));
    assert_eq!(fs::read(workspace.join("shared.txt")).unwrap(), b"first");
}

#[test]
fn parent_swap_after_durable_intent_cannot_redirect_a_mutation() {
    let directory = TestDirectory::new("parent-swap");
    let workspace = directory.workspace();
    fs::create_dir_all(workspace.join("parent")).unwrap();
    fs::write(workspace.join("parent/value.txt"), b"old").unwrap();
    let blobs = BlobStore::open(directory.data()).unwrap();
    let committer = Committer::open(&workspace, blobs, CommitConfig::default()).unwrap();
    let snapshot = committer.snapshot(SnapshotLimits::default()).unwrap();
    let mut vfs = VirtualFs::new(snapshot);
    vfs.write(&path("parent/value.txt"), b"transaction")
        .unwrap();
    let diff = vfs.canonical_diff().unwrap();
    let binding = binding(&vfs, &diff);
    let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
    let store = MemoryTransactionStore::default();
    let reservation = reserve(&store, &binding);
    let swapped = AtomicBool::new(false);

    let result = committer.commit_with_faults(&store, reservation, &plan, &|point| {
        if point == FaultPoint::IntentSynced(0) && !swapped.swap(true, Ordering::AcqRel) {
            fs::rename(workspace.join("parent"), workspace.join("detached-parent")).unwrap();
            fs::create_dir(workspace.join("parent")).unwrap();
            fs::write(workspace.join("parent/value.txt"), b"external").unwrap();
        }
        false
    });

    assert!(matches!(result, Err(CommitError::RecoveryRequired { .. })));
    assert_eq!(
        fs::read(workspace.join("parent/value.txt")).unwrap(),
        b"external"
    );
    assert_eq!(
        fs::read(workspace.join("detached-parent/value.txt")).unwrap(),
        b"transaction"
    );

    let report = committer.recover(&store).unwrap();
    assert_eq!(report.conflicts.len(), 1, "{report:?}");
    assert_eq!(report.conflicts[0].path.as_ref(), Some(&path("parent")));
    assert_eq!(
        report.conflicts[0].reason,
        "recovery parent identity changed"
    );
    assert_eq!(
        fs::read(workspace.join("parent/value.txt")).unwrap(),
        b"external"
    );
    assert_eq!(
        fs::read(workspace.join("detached-parent/value.txt")).unwrap(),
        b"transaction"
    );
}

#[test]
fn every_durable_boundary_is_recoverable_or_already_committed() {
    let mut points = vec![
        FaultPoint::PlanSynced,
        FaultPoint::StageSynced,
        FaultPoint::Revalidated,
        FaultPoint::CommitStatePersisted,
    ];
    for index in 0..4 {
        points.push(FaultPoint::IntentSynced(index));
        points.push(FaultPoint::OperationApplied(index));
        points.push(FaultPoint::DoneSynced(index));
    }
    points.extend([
        FaultPoint::OwnershipMarkersCleared,
        FaultPoint::Verified,
        FaultPoint::CommitMarkerSynced,
        FaultPoint::CommittedStatePersisted,
    ]);

    for (case, point) in points.into_iter().enumerate() {
        let (directory, committer) = fixture(&format!("fault-{case}"));
        let (vfs, diff, binding) = build_fault_transaction(&committer);
        let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
        let store = MemoryTransactionStore::default();
        let reservation = reserve(&store, &binding);
        let result = committer.commit_with_faults(&store, reservation, &plan, &move |candidate| {
            candidate == point
        });

        let state = store.get(binding.transaction_id()).unwrap().state();
        if matches!(
            point,
            FaultPoint::PlanSynced | FaultPoint::StageSynced | FaultPoint::Revalidated
        ) {
            assert!(result.is_err(), "fault {point:?} unexpectedly succeeded");
            assert_eq!(state, TransactionState::Failed, "fault {point:?}");
            assert!(
                workspace_is_original(&directory.workspace()),
                "fault {point:?}"
            );
            continue;
        }

        if point == FaultPoint::CommittedStatePersisted {
            let receipt = result.expect("a durable committed state is success");
            assert!(receipt.cleanup_pending);
            assert_eq!(state, TransactionState::Committed);
        } else {
            assert!(result.is_err(), "fault {point:?} unexpectedly succeeded");
        }

        let report = committer.recover(&store).unwrap();
        assert!(report.conflicts.is_empty(), "fault {point:?}: {report:?}");
        let recovered_state = store.get(binding.transaction_id()).unwrap().state();
        if matches!(
            point,
            FaultPoint::CommitMarkerSynced | FaultPoint::CommittedStatePersisted
        ) {
            assert_eq!(
                recovered_state,
                TransactionState::Committed,
                "fault {point:?}"
            );
            assert!(
                workspace_is_committed(&directory.workspace()),
                "fault {point:?}"
            );
        } else {
            assert_eq!(recovered_state, TransactionState::Failed, "fault {point:?}");
            assert!(
                workspace_is_original(&directory.workspace()),
                "fault {point:?}"
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn recovery_never_follows_a_replaced_internal_plan_symlink() {
    use std::os::unix::fs::symlink;

    let (directory, committer) = fixture("recovery-plan-symlink");
    let (vfs, diff, binding) = build_fault_transaction(&committer);
    let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
    let store = MemoryTransactionStore::default();
    let reservation = reserve(&store, &binding);
    let result = committer.commit_with_faults(&store, reservation, &plan, &|candidate| {
        candidate == FaultPoint::CommitStatePersisted
    });
    assert!(matches!(result, Err(CommitError::RecoveryRequired { .. })));

    let transaction_dir = directory
        .workspace()
        .join(".vsh-runtime/transactions")
        .join(binding.transaction_id().to_string());
    let plan_path = transaction_dir.join(journal::PLAN_FILE);
    let outside = directory.0.join("outside-plan");
    let expected = fs::read(&plan_path).unwrap();
    fs::write(&outside, &expected).unwrap();
    fs::remove_file(&plan_path).unwrap();
    symlink(&outside, &plan_path).unwrap();

    assert!(matches!(
        committer.recover(&store),
        Err(CommitError::InternalIo {
            operation: "open durable commit plan",
            ..
        })
    ));
    assert_eq!(fs::read(outside).unwrap(), expected);
}

#[cfg(unix)]
#[test]
fn incomplete_symlink_install_has_a_durable_staged_inode_witness() {
    use std::os::unix::fs::symlink;

    for (case, point) in [
        FaultPoint::IntentSynced(1),
        FaultPoint::OperationApplied(1),
        FaultPoint::DoneSynced(1),
    ]
    .into_iter()
    .enumerate()
    {
        let directory = TestDirectory::new(&format!("symlink-fault-{case}"));
        fs::create_dir_all(directory.workspace()).unwrap();
        fs::write(directory.workspace().join("old.txt"), b"old").unwrap();
        symlink("old.txt", directory.workspace().join("old-link")).unwrap();
        let blobs = BlobStore::open(directory.data()).unwrap();
        let committer =
            Committer::open(directory.workspace(), blobs, CommitConfig::default()).unwrap();
        let snapshot = committer.snapshot(SnapshotLimits::default()).unwrap();
        let mut vfs = VirtualFs::new(snapshot);
        vfs.rename(&path("old-link"), &path("new-link")).unwrap();
        let diff = vfs.canonical_diff().unwrap();
        let binding = binding(&vfs, &diff);
        let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
        let store = MemoryTransactionStore::default();
        let reservation = reserve(&store, &binding);

        let result = committer.commit_with_faults(&store, reservation, &plan, &move |candidate| {
            candidate == point
        });
        assert!(result.is_err(), "fault {point:?} unexpectedly succeeded");
        let report = committer.recover(&store).unwrap();
        assert!(report.conflicts.is_empty(), "fault {point:?}: {report:?}");
        assert_eq!(
            fs::read_link(directory.workspace().join("old-link")).unwrap(),
            PathBuf::from("old.txt")
        );
        assert!(!directory.workspace().join("new-link").exists());
    }
}
