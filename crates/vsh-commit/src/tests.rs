#[cfg(unix)]
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
    BlobId, FileStamp, NodeKind, NodeState, PlatformFileId, PolicyDigest, ProgramDigest,
    RuntimeConfigDigest, TransactionBinding, TransactionId, TransactionState, VPath,
};
#[cfg(unix)]
use vsh_types::{DiffEntry, DiffKind};
use vsh_vfs::{CanonicalDiff, SnapshotError, VirtualFs};
#[cfg(unix)]
use vsh_vfs::{ReadObservation, WritePrecondition};

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
        invocation: None,
        commit_hook: None,
        execution_evidence: None,
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
    assert!(!vfs.exists(&path(".vsh-runtime")).unwrap());
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

// Windows pins directory handles against rename; its blocked-swap test below
// covers that boundary instead of assuming the relocation succeeds.
#[cfg(unix)]
#[test]
fn relocated_parent_is_rejected_at_mutation_checkpoints() {
    for trigger in [
        FaultPoint::Revalidated,
        FaultPoint::CommitStatePersisted,
        FaultPoint::IntentSynced(0),
    ] {
        let (directory, committer) = fixture("relocated-parent");
        fs::create_dir(directory.workspace().join("sub")).unwrap();
        let outside = directory.0.join("outside");
        fs::create_dir(&outside).unwrap();
        let mut filesystem = VirtualFs::new(committer.snapshot(SnapshotLimits::default()).unwrap());
        filesystem
            .write(&path("sub/created.txt"), b"must stay inside")
            .unwrap();
        let diff = filesystem.canonical_diff().unwrap();
        let binding = binding(&filesystem, &diff);
        let plan = CommitPlan::new(
            &binding,
            &diff,
            filesystem.read_set(),
            filesystem.write_set(),
        )
        .unwrap();
        let store = MemoryTransactionStore::default();
        let reservation = reserve(&store, &binding);
        let result = committer.commit_with_faults(&store, reservation, &plan, &|point| {
            if point == trigger {
                fs::rename(directory.workspace().join("sub"), outside.join("moved")).unwrap();
            }
            false
        });
        assert!(result.is_err());
        assert!(!outside.join("moved/created.txt").exists());
        assert!(!directory.workspace().join("sub/created.txt").exists());
    }
}

#[test]
fn lazy_host_capture_rejects_oversize_growth_and_cancellation_before_blob_storage() {
    let (directory, committer) = fixture("bounded-lazy-capture");
    for limits in [
        SnapshotLimits {
            max_materialized_file_bytes: 2,
            ..SnapshotLimits::default()
        },
        SnapshotLimits {
            max_materialized_bytes: 2,
            ..SnapshotLimits::default()
        },
    ] {
        let snapshot = committer.snapshot(limits).unwrap();
        let mut vfs = VirtualFs::new(snapshot);
        assert!(vfs.read(&path("old.txt")).is_err());
    }
    let cancellation = vsh_execution::ExecutionCancellation::default();
    let snapshot = committer
        .snapshot_cancellable(SnapshotLimits::default(), &cancellation)
        .unwrap();
    assert!(cancellation.cancel());
    assert!(VirtualFs::new(snapshot).read(&path("old.txt")).is_err());
    assert!(
        committer
            .snapshot_cancellable(SnapshotLimits::default(), &cancellation)
            .is_err()
    );

    let snapshot = committer.snapshot(SnapshotLimits::default()).unwrap();
    fs::write(directory.workspace().join("old.txt"), vec![b'x'; 8192]).unwrap();
    assert!(VirtualFs::new(snapshot).read(&path("old.txt")).is_err());
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
        // APFS rejects this filename before VSH can observe it (Darwin EILSEQ).
        // Continue to the special-node check; Linux still exercises NonUtf8Name.
        #[cfg(target_os = "macos")]
        Err(source) if source.raw_os_error() == Some(92) => {}
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

#[cfg(unix)]
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

#[cfg(windows)]
#[test]
fn open_workspace_handle_prevents_relocation() {
    let directory = TestDirectory::new("pinned-workspace");
    let workspace = directory.workspace();
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("visible.txt"), b"original").unwrap();
    let (committer, _data) =
        Committer::open_with_workspace_data(&workspace, CommitConfig::default()).unwrap();
    let relocated = directory.0.join("workspace-relocated");

    assert!(fs::rename(&workspace, &relocated).is_err());
    assert!(!relocated.exists());
    assert!(committer.snapshot(SnapshotLimits::default()).is_ok());
    assert_eq!(
        fs::read(workspace.join("visible.txt")).unwrap(),
        b"original"
    );
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

#[cfg(unix)]
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

#[cfg(target_os = "macos")]
#[test]
fn symlink_replacement_rejects_unrepresentable_modes_before_host_mutation() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let (directory, committer) = fixture("symlink-mode-preflight");
    let workspace = directory.workspace();
    for (name, mode) in [("source", "600"), ("dest", "700")] {
        symlink("old.txt", workspace.join(name)).unwrap();
        assert!(
            std::process::Command::new("/bin/chmod")
                .args(["-h", mode])
                .arg(workspace.join(name))
                .status()
                .unwrap()
                .success()
        );
    }
    let mut vfs = VirtualFs::new(committer.snapshot(SnapshotLimits::default()).unwrap());
    vfs.read_link(&path("dest")).unwrap();
    vfs.rename(&path("source"), &path("dest")).unwrap();
    let diff = vfs.canonical_diff().unwrap();
    let binding = binding(&vfs, &diff);
    let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
    let store = MemoryTransactionStore::default();

    assert!(matches!(
        committer.commit(&store, reserve(&store, &binding), &plan),
        Err(CommitError::Verification(_))
    ));
    assert!(committer.recover(&store).unwrap().conflicts.is_empty());
    for (name, mode) in [("source", 0o600), ("dest", 0o700)] {
        let link = workspace.join(name);
        assert_eq!(fs::read_link(&link).unwrap(), Path::new("old.txt"));
        assert_eq!(
            fs::symlink_metadata(link).unwrap().permissions().mode() & 0o777,
            mode
        );
    }
    assert_eq!(fs::read(workspace.join("old.txt")).unwrap(), b"old");
}

#[cfg(target_os = "macos")]
#[test]
fn same_target_symlink_replacement_quarantines_and_recovers_original_modes() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    for (case, fault) in [
        None,
        Some(FaultPoint::IntentSynced(2)),
        Some(FaultPoint::OperationApplied(2)),
        Some(FaultPoint::DoneSynced(2)),
    ]
    .into_iter()
    .enumerate()
    {
        let (directory, committer) = fixture(&format!("symlink-metadata-replacement-{case}"));
        let workspace = directory.workspace();
        symlink("old.txt", workspace.join("source")).unwrap();
        symlink("old.txt", workspace.join("dest")).unwrap();
        let source_mode = fs::symlink_metadata(workspace.join("source"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        let dest_mode = if source_mode == 0o700 { 0o600 } else { 0o700 };
        assert!(
            std::process::Command::new("/bin/chmod")
                .args(["-h", &format!("{dest_mode:o}")])
                .arg(workspace.join("dest"))
                .status()
                .unwrap()
                .success()
        );
        let mut vfs = VirtualFs::new(committer.snapshot(SnapshotLimits::default()).unwrap());
        vfs.read_link(&path("dest")).unwrap();
        vfs.rename(&path("source"), &path("dest")).unwrap();
        let diff = vfs.canonical_diff().unwrap();
        assert!(
            diff.entries()
                .iter()
                .any(|entry| entry.path == path("dest") && entry.kind == DiffKind::MetadataChange)
        );
        let binding = binding(&vfs, &diff);
        let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
        let store = MemoryTransactionStore::default();
        let result =
            committer.commit_with_faults(&store, reserve(&store, &binding), &plan, &move |point| {
                Some(point) == fault
            });
        if let Some(point) = fault {
            assert!(
                matches!(&result, Err(CommitError::RecoveryRequired { cause, .. }) if cause.contains(&format!("{point:?}"))),
                "{result:?}"
            );
            let report = committer.recover(&store).unwrap();
            assert!(report.conflicts.is_empty(), "{report:?}");
            assert_eq!(
                fs::read_link(workspace.join("source")).unwrap(),
                Path::new("old.txt")
            );
            assert_eq!(
                fs::symlink_metadata(workspace.join("source"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                source_mode
            );
        } else {
            assert!(result.is_ok(), "{result:?}");
            assert!(fs::symlink_metadata(workspace.join("source")).is_err());
        }
        assert_eq!(
            fs::read_link(workspace.join("dest")).unwrap(),
            Path::new("old.txt")
        );
        assert_eq!(
            fs::symlink_metadata(workspace.join("dest"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            if fault.is_none() {
                source_mode
            } else {
                dest_mode
            }
        );
        assert_eq!(fs::read(workspace.join("old.txt")).unwrap(), b"old");
    }
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
        invocation: None,
        commit_hook: None,
        execution_evidence: None,
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

#[cfg(unix)]
#[test]
fn file_permission_commit_preserves_inode_bytes_and_lazy_content() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let (directory, committer) = fixture("file-mode-commit");
    let host_path = directory.workspace().join("old.txt");
    fs::set_permissions(&host_path, fs::Permissions::from_mode(0o644)).unwrap();
    let before = fs::metadata(&host_path).unwrap();
    let snapshot = committer.snapshot(SnapshotLimits::default()).unwrap();
    let mut vfs = VirtualFs::new(snapshot.clone());
    vfs.set_mode(&path("old.txt"), 0o600).unwrap();
    let diff = vfs.canonical_diff().unwrap();
    assert_eq!(diff.entries()[0].kind, DiffKind::MetadataChange);
    assert_eq!(snapshot.metrics().materialized_content_nodes, 0);
    let binding = binding(&vfs, &diff);
    let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
    let store = MemoryTransactionStore::default();
    let receipt = committer
        .commit(&store, reserve(&store, &binding), &plan)
        .unwrap();
    assert_eq!(receipt.operations, 1);
    let after = fs::metadata(&host_path).unwrap();
    assert_eq!(after.ino(), before.ino());
    assert_eq!(after.dev(), before.dev());
    assert_eq!(after.mtime(), before.mtime());
    assert_eq!(after.mtime_nsec(), before.mtime_nsec());
    assert_eq!(after.mode() & 0o7777, 0o600);
    assert_eq!(fs::read(&host_path).unwrap(), b"old");
    assert_eq!(snapshot.metrics().materialized_content_nodes, 0);
}

#[cfg(unix)]
#[test]
fn file_permission_commit_rejects_stale_content_or_mode() {
    use std::os::unix::fs::PermissionsExt;
    for content_change in [false, true] {
        let (directory, committer) = fixture("file-mode-stale");
        let host_path = directory.workspace().join("old.txt");
        fs::set_permissions(&host_path, fs::Permissions::from_mode(0o644)).unwrap();
        let mut vfs = VirtualFs::new(committer.snapshot(SnapshotLimits::default()).unwrap());
        vfs.set_mode(&path("old.txt"), 0o600).unwrap();
        let diff = vfs.canonical_diff().unwrap();
        let binding = binding(&vfs, &diff);
        let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
        if content_change {
            fs::write(&host_path, b"external").unwrap();
        } else {
            fs::set_permissions(&host_path, fs::Permissions::from_mode(0o640)).unwrap();
        }
        let store = MemoryTransactionStore::default();
        assert!(matches!(
            committer.commit(&store, reserve(&store, &binding), &plan),
            Err(CommitError::Stale { .. })
        ));
        assert_eq!(
            fs::read(&host_path).unwrap(),
            if content_change {
                b"external".as_slice()
            } else {
                b"old".as_slice()
            }
        );
        assert_eq!(
            fs::metadata(&host_path).unwrap().permissions().mode() & 0o777,
            if content_change { 0o644 } else { 0o640 }
        );
    }
}

#[cfg(unix)]
#[test]
fn file_permission_commits_recover_at_every_durable_boundary() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    for point in [
        FaultPoint::PlanSynced,
        FaultPoint::StageSynced,
        FaultPoint::Revalidated,
        FaultPoint::CommitStatePersisted,
        FaultPoint::IntentSynced(0),
        FaultPoint::OperationApplied(0),
        FaultPoint::DoneSynced(0),
        FaultPoint::OwnershipMarkersCleared,
        FaultPoint::Verified,
        FaultPoint::CommitMarkerSynced,
        FaultPoint::CommittedStatePersisted,
    ] {
        let (directory, committer) = fixture("file-mode-fault");
        let host_path = directory.workspace().join("old.txt");
        fs::set_permissions(&host_path, fs::Permissions::from_mode(0o644)).unwrap();
        let inode = fs::metadata(&host_path).unwrap().ino();
        let mut vfs = VirtualFs::new(committer.snapshot(SnapshotLimits::default()).unwrap());
        vfs.set_mode(&path("old.txt"), 0o600).unwrap();
        let diff = vfs.canonical_diff().unwrap();
        let binding = binding(&vfs, &diff);
        let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
        let store = MemoryTransactionStore::default();
        let result = committer.commit_with_faults(
            &store,
            reserve(&store, &binding),
            &plan,
            &move |candidate| candidate == point,
        );
        if point != FaultPoint::CommittedStatePersisted {
            assert!(result.is_err(), "{point:?}");
        }
        let report = committer.recover(&store).unwrap();
        assert!(report.conflicts.is_empty(), "{point:?}: {report:?}");
        let completed = matches!(
            point,
            FaultPoint::CommitMarkerSynced | FaultPoint::CommittedStatePersisted
        );
        assert_eq!(
            fs::metadata(&host_path).unwrap().permissions().mode() & 0o777,
            if completed { 0o600 } else { 0o644 },
            "{point:?}"
        );
        assert_eq!(fs::metadata(&host_path).unwrap().ino(), inode);
        assert_eq!(fs::read(&host_path).unwrap(), b"old");
    }
}

#[cfg(unix)]
#[test]
fn file_permission_commits_reject_existing_and_racing_hard_link_aliases() {
    use std::os::unix::fs::PermissionsExt;
    for existing in [true, false] {
        let (directory, committer) = fixture("mode-hard-link");
        let original = directory.workspace().join("old.txt");
        let alias = directory.0.join("unapproved-alias.txt");
        fs::set_permissions(&original, fs::Permissions::from_mode(0o644)).unwrap();
        if existing {
            fs::hard_link(&original, &alias).unwrap();
        }
        let mut vfs = VirtualFs::new(committer.snapshot(SnapshotLimits::default()).unwrap());
        vfs.set_mode(&path("old.txt"), 0o600).unwrap();
        let diff = vfs.canonical_diff().unwrap();
        let binding = binding(&vfs, &diff);
        let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
        let store = MemoryTransactionStore::default();
        assert!(
            committer
                .commit_with_faults(&store, reserve(&store, &binding), &plan, &|point| {
                    if !existing && point == FaultPoint::IntentSynced(0) {
                        fs::hard_link(&original, &alias).unwrap();
                    }
                    false
                })
                .is_err()
        );
        for candidate in [&original, &alias] {
            assert_eq!(
                fs::metadata(candidate).unwrap().permissions().mode() & 0o777,
                0o644
            );
            assert_eq!(fs::read(candidate).unwrap(), b"old");
        }
    }
}

#[cfg(unix)]
#[test]
fn inaccessible_mode_transitions_are_rejected_before_host_mutation() {
    use std::os::unix::fs::PermissionsExt;
    for (before, after) in [(0o644, 0), (0, 0o644), (0o644, 0o111)] {
        let (directory, committer) = fixture("mode-no-access");
        let host_path = directory.workspace().join("old.txt");
        fs::set_permissions(&host_path, fs::Permissions::from_mode(before)).unwrap();
        let mut vfs = VirtualFs::new(committer.snapshot(SnapshotLimits::default()).unwrap());
        vfs.set_mode(&path("old.txt"), after).unwrap();
        let diff = vfs.canonical_diff().unwrap();
        let binding = binding(&vfs, &diff);
        assert!(matches!(
            CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()),
            Err(CommitPlanError::UnsupportedMetadataMode { .. })
        ));
        assert_eq!(
            fs::metadata(&host_path).unwrap().permissions().mode() & 0o777,
            before
        );
        fs::set_permissions(&host_path, fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(fs::read(&host_path).unwrap(), b"old");
    }
    for (created, mode) in [
        (false, 0),
        (false, 0o400),
        (true, 0),
        (true, 0o500),
        (true, 0o555),
    ] {
        let (directory, committer) = fixture("directory-mode-no-access");
        let host_path = directory.workspace().join("folder");
        if !created {
            fs::create_dir(&host_path).unwrap();
            fs::set_permissions(&host_path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut vfs = VirtualFs::new(committer.snapshot(SnapshotLimits::default()).unwrap());
        if created {
            vfs.mkdir(&path("folder"), mode).unwrap();
        } else {
            vfs.set_mode(&path("folder"), mode).unwrap();
        }
        let diff = vfs.canonical_diff().unwrap();
        let binding = binding(&vfs, &diff);
        assert!(matches!(
            CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()),
            Err(CommitPlanError::UnsupportedMetadataMode { .. })
        ));
        assert_eq!(host_path.exists(), !created);
        if !created {
            assert_eq!(
                fs::metadata(&host_path).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn write_only_file_modes_can_be_recovered_without_content_read_access() {
    use std::os::unix::fs::PermissionsExt;
    let (directory, committer) = fixture("mode-write-only");
    let host_path = directory.workspace().join("old.txt");
    fs::set_permissions(&host_path, fs::Permissions::from_mode(0o644)).unwrap();
    let mut vfs = VirtualFs::new(committer.snapshot(SnapshotLimits::default()).unwrap());
    vfs.set_mode(&path("old.txt"), 0o200).unwrap();
    let diff = vfs.canonical_diff().unwrap();
    let binding = binding(&vfs, &diff);
    let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
    let store = MemoryTransactionStore::default();
    assert!(
        committer
            .commit_with_faults(&store, reserve(&store, &binding), &plan, &|point| point
                == FaultPoint::DoneSynced(0))
            .is_err()
    );
    assert_eq!(
        fs::metadata(&host_path).unwrap().permissions().mode() & 0o777,
        0o200
    );
    let report = committer.recover(&store).unwrap();
    assert!(report.conflicts.is_empty(), "{report:?}");
    assert_eq!(
        fs::metadata(&host_path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    assert_eq!(fs::read(&host_path).unwrap(), b"old");
}

#[cfg(unix)]
#[test]
fn completed_mode_rollback_is_restart_idempotent() {
    use std::os::unix::fs::PermissionsExt;
    let (directory, committer) = fixture("mode-rollback-restart");
    let host_path = directory.workspace().join("old.txt");
    fs::set_permissions(&host_path, fs::Permissions::from_mode(0o644)).unwrap();
    let mut vfs = VirtualFs::new(committer.snapshot(SnapshotLimits::default()).unwrap());
    vfs.set_mode(&path("old.txt"), 0o600).unwrap();
    let diff = vfs.canonical_diff().unwrap();
    let binding = binding(&vfs, &diff);
    let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
    let store = MemoryTransactionStore::default();
    assert!(
        committer
            .commit_with_faults(&store, reserve(&store, &binding), &plan, &|point| point
                == FaultPoint::DoneSynced(0))
            .is_err()
    );
    // Emulate loss after permission restoration but before store transition/cleanup.
    fs::set_permissions(&host_path, fs::Permissions::from_mode(0o644)).unwrap();
    let report = committer.recover(&store).unwrap();
    assert!(report.conflicts.is_empty(), "{report:?}");
    assert_eq!(report.rolled_back, 1);
    assert_eq!(fs::read(&host_path).unwrap(), b"old");
    assert_eq!(
        fs::metadata(&host_path).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

#[cfg(unix)]
#[test]
fn blob_backed_mode_commits_bind_the_intent_to_the_original_file_identity() {
    use std::os::unix::fs::PermissionsExt;
    for replace_after_intent in [false, true] {
        let (directory, committer) = fixture("mode-blob-intent");
        let host_path = directory.workspace().join("old.txt");
        fs::set_permissions(&host_path, fs::Permissions::from_mode(0o644)).unwrap();
        let root =
            cap_std::fs::Dir::open_ambient_dir(directory.workspace(), cap_std::ambient_authority())
                .unwrap();
        let mut snapshot = vsh_vfs::SnapshotBuilder::with_root_stamp(
            committer.artifact_store(),
            host::stamp_dir(&root, &VPath::root()).unwrap(),
        );
        snapshot.add_file(path("old.txt"), b"old", 0o644).unwrap();
        let mut vfs = VirtualFs::new(snapshot.build().unwrap());
        vfs.set_mode(&path("old.txt"), 0o600).unwrap();
        let diff = vfs.canonical_diff().unwrap();
        let binding = binding(&vfs, &diff);
        let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
        let store = MemoryTransactionStore::default();
        let result =
            committer.commit_with_faults(&store, reserve(&store, &binding), &plan, &|point| {
                if replace_after_intent && point == FaultPoint::IntentSynced(0) {
                    fs::rename(&host_path, directory.0.join("original.txt")).unwrap();
                    fs::write(&host_path, b"old").unwrap();
                    fs::set_permissions(&host_path, fs::Permissions::from_mode(0o644)).unwrap();
                }
                false
            });
        assert_eq!(result.is_err(), replace_after_intent, "{result:?}");
        assert_eq!(
            fs::metadata(&host_path).unwrap().permissions().mode() & 0o777,
            if replace_after_intent { 0o644 } else { 0o600 }
        );
        assert_eq!(fs::read(&host_path).unwrap(), b"old");
        if replace_after_intent {
            let report = committer.recover(&store).unwrap();
            assert_eq!(report.conflicts.len(), 1, "{report:?}");
            assert_eq!(
                fs::metadata(&host_path).unwrap().permissions().mode() & 0o777,
                0o644
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn unreadable_blob_backed_mode_evidence_is_rejected_before_chmod() {
    use std::os::unix::fs::PermissionsExt;
    let (directory, committer) = fixture("mode-blob-unreadable");
    let host_path = directory.workspace().join("old.txt");
    fs::set_permissions(&host_path, fs::Permissions::from_mode(0o644)).unwrap();
    let root =
        cap_std::fs::Dir::open_ambient_dir(directory.workspace(), cap_std::ambient_authority())
            .unwrap();
    let mut snapshot = vsh_vfs::SnapshotBuilder::with_root_stamp(
        committer.artifact_store(),
        host::stamp_dir(&root, &VPath::root()).unwrap(),
    );
    snapshot.add_file(path("old.txt"), b"old", 0o644).unwrap();
    let mut vfs = VirtualFs::new(snapshot.build().unwrap());
    vfs.set_mode(&path("old.txt"), 0o200).unwrap();
    let diff = vfs.canonical_diff().unwrap();
    let binding = binding(&vfs, &diff);
    assert!(matches!(
        CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()),
        Err(CommitPlanError::UnsupportedMetadataMode { .. })
    ));
    assert_eq!(
        fs::metadata(&host_path).unwrap().permissions().mode() & 0o777,
        0o644
    );
    assert_eq!(fs::read(&host_path).unwrap(), b"old");
}

#[cfg(unix)]
#[test]
fn directory_permission_changes_follow_child_writes_and_recover_before_child_undo() {
    use std::os::unix::fs::PermissionsExt;
    for interrupted in [false, true] {
        let (directory, committer) = fixture("directory-mode-with-child-write");
        let folder = directory.workspace().join("folder");
        fs::create_dir(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        let mut vfs = VirtualFs::new(committer.snapshot(SnapshotLimits::default()).unwrap());
        vfs.set_mode(&path("folder"), 0o555).unwrap();
        vfs.write(&path("folder/new.txt"), b"new").unwrap();
        let diff = vfs.canonical_diff().unwrap();
        let binding = binding(&vfs, &diff);
        let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
        let store = MemoryTransactionStore::default();
        let result =
            committer.commit_with_faults(&store, reserve(&store, &binding), &plan, &|point| {
                interrupted && point == FaultPoint::OperationApplied(1)
            });
        assert_eq!(result.is_err(), interrupted, "{result:?}");
        assert_eq!(
            fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
            0o555
        );
        let report = committer.recover(&store).unwrap();
        assert!(report.conflicts.is_empty(), "{report:?}");
        assert_eq!(folder.join("new.txt").exists(), !interrupted);
        assert_eq!(
            fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
            if interrupted { 0o755 } else { 0o555 }
        );
        if !interrupted {
            assert_eq!(fs::read(folder.join("new.txt")).unwrap(), b"new");
        }
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn directory_access_grants_precede_nested_installs_and_roll_back_last() {
    use std::os::unix::fs::PermissionsExt;
    for fault in [
        None,
        Some(FaultPoint::DoneSynced(0)),
        Some(FaultPoint::DoneSynced(1)),
        Some(FaultPoint::OperationApplied(2)),
    ] {
        let (directory, committer) = fixture("directory-access-grant");
        let folder = directory.workspace().join("folder");
        let nested = folder.join("nested");
        fs::create_dir_all(&nested).unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o500)).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o555)).unwrap();
        let mut vfs = VirtualFs::new(committer.snapshot(SnapshotLimits::default()).unwrap());
        vfs.set_mode(&path("folder"), 0o755).unwrap();
        vfs.set_mode(&path("folder/nested"), 0o750).unwrap();
        vfs.write(&path("folder/nested/new.txt"), b"new").unwrap();
        let diff = vfs.canonical_diff().unwrap();
        let binding = binding(&vfs, &diff);
        let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
        let store = MemoryTransactionStore::default();
        let result =
            committer.commit_with_faults(&store, reserve(&store, &binding), &plan, &|point| {
                fault == Some(point)
            });
        assert_eq!(result.is_err(), fault.is_some(), "{result:?}");
        let report = committer.recover(&store).unwrap();
        assert!(report.conflicts.is_empty(), "{report:?}");
        assert_eq!(nested.join("new.txt").exists(), fault.is_none());
        assert_eq!(
            fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
            if fault.is_some() { 0o555 } else { 0o755 }
        );
        assert_eq!(
            fs::metadata(&nested).unwrap().permissions().mode() & 0o777,
            if fault.is_some() { 0o500 } else { 0o750 }
        );
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&nested, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn completed_directory_mode_recovery_preserves_third_modes_and_already_undone_state() {
    use std::os::unix::fs::PermissionsExt;
    for external_mode in [0o700, 0o755] {
        let (directory, committer) = fixture("directory-mode-recovery-state");
        let folder = directory.workspace().join("folder");
        fs::create_dir(&folder).unwrap();
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
        let mut vfs = VirtualFs::new(committer.snapshot(SnapshotLimits::default()).unwrap());
        vfs.set_mode(&path("folder"), 0o555).unwrap();
        let diff = vfs.canonical_diff().unwrap();
        let binding = binding(&vfs, &diff);
        let plan = CommitPlan::new(&binding, &diff, vfs.read_set(), vfs.write_set()).unwrap();
        let store = MemoryTransactionStore::default();
        assert!(
            committer
                .commit_with_faults(&store, reserve(&store, &binding), &plan, &|point| point
                    == FaultPoint::DoneSynced(0))
                .is_err()
        );
        fs::set_permissions(&folder, fs::Permissions::from_mode(external_mode)).unwrap();
        let report = committer.recover(&store).unwrap();
        assert_eq!(
            report.conflicts.is_empty(),
            external_mode == 0o755,
            "{report:?}"
        );
        assert_eq!(report.rolled_back, usize::from(external_mode == 0o755));
        assert_eq!(
            fs::metadata(&folder).unwrap().permissions().mode() & 0o777,
            external_mode
        );
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).unwrap();
    }
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

#[cfg(unix)]
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
        b"old"
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
        b"old"
    );
}

#[cfg(windows)]
#[test]
fn pinned_parent_handle_prevents_swap_during_commit() {
    let directory = TestDirectory::new("parent-swap-blocked");
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
    let attempted = AtomicBool::new(false);
    let blocked = AtomicBool::new(false);

    let receipt = committer
        .commit_with_faults(&store, reservation, &plan, &|point| {
            if point == FaultPoint::IntentSynced(0) && !attempted.swap(true, Ordering::AcqRel) {
                blocked.store(
                    fs::rename(workspace.join("parent"), workspace.join("detached-parent"))
                        .is_err(),
                    Ordering::Release,
                );
            }
            false
        })
        .unwrap();

    assert!(attempted.load(Ordering::Acquire));
    assert!(blocked.load(Ordering::Acquire));
    assert!(!receipt.cleanup_pending);
    assert_eq!(
        fs::read(workspace.join("parent/value.txt")).unwrap(),
        b"transaction"
    );
    assert!(!workspace.join("detached-parent").exists());
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
            assert!(!receipt.cleanup_pending);
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
