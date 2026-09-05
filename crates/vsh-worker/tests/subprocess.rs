//! End-to-end tests for the bundled crash-isolated Monty worker.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use vsh_monty::{
    ExecutionError, ExecutionLimitExceeded, ExecutionLimits, InProcessConfig, MontyObject,
    SubprocessConfig, SubprocessMonty, WorkerFailureKind,
};
use vsh_store::BlobStore;
use vsh_types::{DiffKind, VPath};
use vsh_vfs::{SnapshotBuilder, VirtualFs};

static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let sequence = TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "vsh-worker-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("test directory should be unique");
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

fn worker_path() -> &'static str {
    env!("CARGO_BIN_EXE_vsh-monty-worker")
}

fn make_filesystem(files: &[(&str, &[u8])]) -> (TestDirectory, VirtualFs) {
    let directory = TestDirectory::new("vfs");
    let store = BlobStore::open(directory.path()).expect("blob store should open");
    let mut builder = SnapshotBuilder::new(store);
    for (path, bytes) in files {
        builder
            .add_file(
                VPath::parse(path).expect("test path should parse"),
                bytes,
                0o644,
            )
            .expect("test file should be added");
    }
    let snapshot = builder.build().expect("snapshot should build");
    (directory, VirtualFs::new(snapshot))
}

fn engine(config: InProcessConfig) -> SubprocessMonty {
    SubprocessMonty::new(
        SubprocessConfig::new(worker_path(), config)
            .with_wall_timeout(Duration::from_secs(3))
            .with_max_idle_workers(1),
    )
    .expect("bundled worker should validate")
}

#[test]
fn typed_vfs_execution_is_exact_and_clean_worker_is_reused() {
    let engine = engine(InProcessConfig::default());
    let (_directory, mut filesystem) = make_filesystem(&[("input.txt", b"hello\n")]);
    let outcome = engine
        .execute(
            r"
from pathlib import Path
source = Path('/workspace/input.txt').read_text()
Path('/workspace/out').mkdir()
Path('/workspace/out/result.txt').write_text(source.upper())
Path('/workspace/input.txt').rename('/workspace/archive.txt')
len(source)
",
            &mut filesystem,
        )
        .expect("worker program should execute");

    assert_eq!(outcome.value, MontyObject::Int(6));
    assert_eq!(outcome.stats.os_calls, 4);
    assert_eq!(outcome.stats.read_bytes, 6);
    assert_eq!(outcome.stats.write_bytes, 6);
    assert_eq!(engine.idle_workers().unwrap(), 1);

    let diff = filesystem.canonical_diff().unwrap();
    let changes = diff
        .entries()
        .iter()
        .map(|entry| (entry.path.as_str(), entry.kind))
        .collect::<Vec<_>>();
    assert_eq!(
        changes,
        vec![
            ("archive.txt", DiffKind::Create),
            ("input.txt", DiffKind::Delete),
            ("out", DiffKind::Create),
            ("out/result.txt", DiffKind::Create),
        ]
    );

    let (_second_directory, mut second_filesystem) = make_filesystem(&[]);
    let second = engine
        .execute("40 + 2", &mut second_filesystem)
        .expect("reset worker should execute another program");
    assert_eq!(second.value, MontyObject::Int(42));
    assert_eq!(engine.idle_workers().unwrap(), 1);
}

#[test]
fn subprocess_vsh_tools_share_the_active_overlay_with_pathlib() {
    let engine = engine(InProcessConfig::default());
    let (_directory, mut filesystem) = make_filesystem(&[("input.txt", b"hello\n")]);
    let outcome = engine
        .execute(
            r"
from pathlib import Path
source = vsh_read('/workspace/input.txt')
vsh_mkdir('/workspace/out')
vsh_write('/workspace/out/value.txt', source.upper())
Path('/workspace/out/value.txt').read_text()
",
            &mut filesystem,
        )
        .expect("worker should dispatch VSH functions against its caller's overlay");

    assert_eq!(outcome.value, MontyObject::String("HELLO\n".to_owned()));
    assert_eq!(outcome.stats.os_calls, 4);
    assert_eq!(
        filesystem
            .read(&VPath::parse("out/value.txt").unwrap())
            .unwrap(),
        b"HELLO\n"
    );
    assert_eq!(engine.idle_workers().unwrap(), 1);
}

#[test]
fn subprocess_vsh_tool_payload_uses_the_typed_call_frame_budget() {
    const PAYLOAD_BYTES: usize = 1_100_000;
    let limits = ExecutionLimits {
        max_io_call_bytes: 2 * 1024 * 1024,
        max_write_bytes: 2 * 1024 * 1024,
        ..ExecutionLimits::default()
    };
    let engine = engine(InProcessConfig::default().with_limits(limits));
    let (_directory, mut filesystem) = make_filesystem(&[]);
    let outcome = engine
        .execute(
            format!("vsh_write('/workspace/large.txt', 'x' * {PAYLOAD_BYTES})"),
            &mut filesystem,
        )
        .expect("a valid VSH function payload may exceed the program frame budget");

    assert_eq!(
        outcome.value,
        MontyObject::Int(i64::try_from(PAYLOAD_BYTES).unwrap())
    );
    let payload_bytes = u64::try_from(PAYLOAD_BYTES).unwrap();
    assert_eq!(outcome.stats.write_bytes, payload_bytes);
    assert_eq!(
        filesystem
            .metadata(&VPath::parse("large.txt").unwrap())
            .unwrap()
            .size(),
        payload_bytes
    );
}

#[test]
fn native_runtime_uses_worker_for_an_exact_auto_commit() {
    let directory = TestDirectory::new("runtime");
    fs::write(directory.path().join("input.txt"), b"hello\n").unwrap();
    let runtime = vsh::Runtime::open(
        vsh::RuntimeConfig::new(directory.path())
            .with_worker_path(worker_path())
            .with_max_idle_workers(1),
    )
    .expect("runtime should validate the bundled worker");
    let receipt = runtime
        .run(
            vsh::RunRequest::new(
                r"
from pathlib import Path
value = Path('/workspace/input.txt').read_text()
Path('/workspace/output.txt').write_text(value.upper())
len(value)
",
            )
            .with_mode(vsh::RunMode::Auto)
            .with_detail(vsh::ReceiptDetail::Full),
        )
        .expect("worker-backed runtime should commit");

    assert_eq!(receipt.state, vsh::TransactionState::Committed);
    assert!(matches!(
        receipt.decision,
        vsh::RuntimeDecision::AutoApproved
    ));
    assert_eq!(receipt.value, MontyObject::Int(6));
    assert_eq!(
        fs::read(directory.path().join("output.txt")).unwrap(),
        b"HELLO\n"
    );
}

#[test]
fn caught_protected_read_is_reported_without_touching_secret() {
    let engine = engine(InProcessConfig::default());
    let (_directory, mut filesystem) = make_filesystem(&[(".env", b"TOKEN=host-secret\n")]);
    let outcome = engine
        .execute(
            r"
from pathlib import Path
try:
    Path('/workspace/.env').read_text()
except PermissionError:
    Path('/workspace/safe.txt').write_text('continued')
'done'
",
            &mut filesystem,
        )
        .expect("sandboxed code may catch the policy exception");

    assert_eq!(outcome.value, MontyObject::String("done".to_owned()));
    assert_eq!(outcome.stats.read_bytes, 0);
    assert_eq!(outcome.stats.denied_accesses, 1);
    assert_eq!(outcome.denied_accesses[0].path.as_str(), ".env");
    assert!(
        !filesystem
            .read_set()
            .contains_key(&VPath::parse(".env").unwrap())
    );
    assert_eq!(engine.idle_workers().unwrap(), 1);
}

#[test]
fn output_limit_discards_worker_before_reuse() {
    let limits = ExecutionLimits {
        max_output_bytes: 16,
        ..ExecutionLimits::default()
    };
    let engine = engine(InProcessConfig::default().with_limits(limits));
    let (_directory, mut filesystem) = make_filesystem(&[]);
    let error = engine
        .execute("print('x' * 1_000)", &mut filesystem)
        .expect_err("oversized output must be rejected");
    assert!(matches!(
        error,
        ExecutionError::Limit(source)
            if matches!(*source, ExecutionLimitExceeded::OutputBytes { limit: 16, .. })
    ));
    assert_eq!(engine.idle_workers().unwrap(), 0);

    let (_second_directory, mut second_filesystem) = make_filesystem(&[]);
    let outcome = engine
        .execute("6 * 7", &mut second_filesystem)
        .expect("a fresh worker should execute after discard");
    assert_eq!(outcome.value, MontyObject::Int(42));
    assert_eq!(engine.idle_workers().unwrap(), 1);
}

#[test]
fn oversized_result_frame_is_rejected_before_protobuf_object_decode() {
    let limits = ExecutionLimits {
        max_result_bytes: 128,
        ..ExecutionLimits::default()
    };
    let engine = engine(InProcessConfig::default().with_limits(limits));
    let (_directory, mut filesystem) = make_filesystem(&[]);
    let error = engine
        .execute("'x' * 1_000_000", &mut filesystem)
        .expect_err("oversized result must be rejected at the frame boundary");
    assert!(matches!(
        error,
        ExecutionError::Limit(source)
            if matches!(*source, ExecutionLimitExceeded::ResultBytes { limit: 128, .. })
    ));
    assert_eq!(engine.idle_workers().unwrap(), 0);
}

#[test]
fn tiny_print_event_flood_is_bounded_independently_from_output_bytes() {
    let limits = ExecutionLimits {
        max_duration: Duration::from_secs(10),
        ..ExecutionLimits::default()
    };
    let engine = SubprocessMonty::new(
        SubprocessConfig::new(
            worker_path(),
            InProcessConfig::default().with_limits(limits),
        )
        .with_wall_timeout(Duration::from_secs(10))
        .with_max_idle_workers(1),
    )
    .expect("bundled worker should validate");
    let (_directory, mut filesystem) = make_filesystem(&[]);
    let error = engine
        .execute("for _ in range(20_000):\n    print()", &mut filesystem)
        .expect_err("small print frames must not create an unbounded event stream");
    assert!(matches!(
        error,
        ExecutionError::Worker(source) if source.kind == WorkerFailureKind::Protocol
    ));
    assert_eq!(engine.idle_workers().unwrap(), 0);
}

#[test]
fn wall_timeout_kills_worker_and_next_execution_starts_cleanly() {
    let limits = ExecutionLimits {
        max_duration: Duration::from_secs(10),
        ..ExecutionLimits::default()
    };
    let engine = SubprocessMonty::new(
        SubprocessConfig::new(
            worker_path(),
            InProcessConfig::default().with_limits(limits),
        )
        .with_wall_timeout(Duration::from_millis(250))
        .with_max_idle_workers(1),
    )
    .expect("bundled worker should validate");
    let (_directory, mut filesystem) = make_filesystem(&[]);
    let error = engine
        .execute("while True:\n    pass", &mut filesystem)
        .expect_err("watchdog must terminate an infinite program");
    assert!(matches!(
        error,
        ExecutionError::Worker(source) if source.kind == WorkerFailureKind::Timeout
    ));
    assert_eq!(engine.idle_workers().unwrap(), 0);

    let (_second_directory, mut second_filesystem) = make_filesystem(&[]);
    let outcome = engine
        .execute("21 * 2", &mut second_filesystem)
        .expect("a fresh worker should execute after timeout");
    assert_eq!(outcome.value, MontyObject::Int(42));
}

#[cfg(unix)]
#[test]
fn wrong_worker_version_is_rejected_before_execution() {
    use std::os::unix::fs::PermissionsExt;

    let directory = TestDirectory::new("wrong-version");
    let executable = directory.path().join("worker");
    fs::write(
        &executable,
        "#!/bin/sh\nprintf 'vsh-monty-worker 0.0.20\\n'\n",
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();

    let error = SubprocessMonty::new(SubprocessConfig::new(
        &executable,
        InProcessConfig::default(),
    ))
    .expect_err("wrong Monty version must fail closed");
    assert!(matches!(
        error,
        ExecutionError::Worker(source) if source.kind == WorkerFailureKind::Spawn
    ));
}
