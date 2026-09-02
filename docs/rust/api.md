# Rust API reference

This page documents the application-facing `vsh` facade. Rustdoc remains the
exhaustive source for every re-exported lower-level field and error variant.

## Constants and diagnostics

```rust
pub const VERSION: &str;
pub const fn engine_kind() -> &'static str;
```

`VERSION` is shared with the Python wheel and worker. `engine_kind()` returns `"rust"`.

## `RuntimeConfig`

```rust
let config = RuntimeConfig::new(workspace_root)
    .with_data_directory(trusted_data_root)
    .with_worker_path(worker)
    .with_max_idle_workers(4)
    .with_policy_profile(PolicyProfile::Strict)
    .with_result_compatibility(ResultCompatibility::Native);
```

| Builder/accessor | Purpose |
|---|---|
| `new(workspace_root)` | Balanced configuration with protected `.vsh-runtime/data` |
| `with_data_directory(path)` | Select a trusted external durable-data capability |
| `with_worker_path(path)` | Select the exact supervised worker executable |
| `with_max_idle_workers(count)` | Bound reusable clean workers; zero disables pooling |
| `with_result_compatibility(kind)` | Require native or Python-projectable result values |
| `with_in_process_execution()` | Trusted-only correctness/benchmark harness |
| `with_virtual_root(root)` | Change the synthetic absolute namespace shown to Monty |
| `with_policy(policy)` | Supply a custom validated deterministic policy |
| `with_policy_profile(profile)` | Select balanced, strict, or paranoid presets |
| `with_snapshot_limits(limits)` | Replace eager traversal and snapshot ceilings |
| `with_commit_config(config)` | Replace trusted commit/recovery ceilings |
| `with_store_config(config)` | Replace durable transaction-log ceilings |
| `with_artifact_limits(limits)` | Replace pending-artifact and cache ceilings |
| `workspace_root()` | Return the host workspace root |
| `data_directory()` | Return the trusted artifact root |
| `worker_path()` | Return the worker path or `None` in trusted in-process mode |
| `policy()` | Borrow the deterministic transaction policy |

Builder values contribute to runtime configuration identity where security relevant.

## `RunRequest<'a>`

```rust
pub struct RunRequest<'a> {
    pub code: &'a str,
    pub intent: Option<&'a str>,
    pub mode: RunMode,
    pub detail: ReceiptDetail,
    pub budget: ExecutionBudget,
}
```

`RunRequest::new(code)` creates a compact preview with default limits. Chain
`with_intent`, `with_mode`, `with_detail`, and `with_budget` to replace one choice.

## Modes and detail

```rust
pub enum RunMode { Preview, Auto }
pub enum ReceiptDetail { Compact, Full }
```

Preview never changes the host. Auto commits only deterministic `AutoApproved` work.
Compact omits per-path entries while retaining identity and counts; Full retains the
complete bounded canonical diff.

## `Runtime`

| Method | Result | Contract |
|---|---|---|
| `Runtime::open(config)` | `Runtime` | Establish capabilities, stores, worker supervision, and startup recovery |
| `startup_recovery()` | `&RecoveryReport` | Inspect recovery performed during open |
| `run(request)` | `Receipt` | Execute according to request mode |
| `preview(request)` | `Receipt` | Force a non-mutating call |
| `discard_preview(id)` | `bool` | Release a process-local auto-approved artifact |
| `approve(id, principal, issued, expires)` | `TransactionRecord` | Bind an independent time-limited approval |
| `commit(id, now)` | `Receipt` | Single-use reserve, revalidate, apply, verify |
| `recover()` | `RecoveryReport` | Resolve bounded durable commit state |
| `transaction(id)` | `TransactionRecord` | Read current durable lifecycle state |

All fallible methods return `Result<_, VshError>`.

## `Receipt`

```rust
pub struct Receipt {
    pub transaction: TransactionId,
    pub base_snapshot: SnapshotId,
    pub state: TransactionState,
    pub decision: RuntimeDecision,
    pub diff: DiffDigest,
    pub changed_paths: usize,
    pub changes: Vec<DiffEntry>,
    pub value: MontyObject,
    pub stdout: String,
    pub execution: ExecutionStats,
    pub timings: StageTimings,
    pub commit: Option<CommitReceipt>,
}
```

`RuntimeDecision` is one of:

- `Denied(DenyManifest)`;
- `AutoApproved`;
- `PendingApproval(RiskManifest)`.

Pattern-match the decision rather than parsing its display text.

## `StageTimings`

All fields are nanoseconds measured with a monotonic clock:

| Field | Includes |
|---|---|
| `snapshot_ns` | Capability-rooted metadata/content snapshot |
| `execute_ns` | Monty bytecode and typed VirtualFs calls |
| `diff_ns` | Canonical diff freeze |
| `policy_ns` | Deterministic policy evaluation |
| `bind_and_store_ns` | Artifact binding and lifecycle writes |
| `commit_ns` | Reservation, revalidation, application, verification |
| `total_ns` | Complete native runtime call |

## `ExecutionBudget`

`ExecutionBudget` aliases `vsh_monty::ExecutionLimits` and has public fields for source
bytes, duration, recursion, memory, OS calls, read/write bytes, per-call I/O, path bytes,
directory entries, stdout, returned value, and exception payload. Prefer struct update
syntax from `Default` so later compatible fields receive safe defaults.

## `ArtifactLimits`

Trusted-host bounds for durable pending artifacts:

- total encoded artifact bytes;
- encoded result and retained stdout;
- canonical entries and dependencies;
- one-path bytes;
- process-local preview count and aggregate encoded bytes.

These do not replace execution budgets; they constrain the artifact VSH retains after
execution.

## `VshError`

`VshError` is `#[non_exhaustive]`. Important categories include data-directory,
blob-store, commit, execution, VFS, state-store, approval, artifact, result-compatibility,
unsafe-overlap, binding mismatch, recovery conflict, missing/duplicate pending artifact,
ephemeral capacity, and poisoned pending state.

Match categories you can handle and keep a fallback arm:

```rust
match runtime.commit(transaction, now_ms) {
    Ok(receipt) => consume(receipt),
    Err(vsh::VshError::Commit(error)) => report_commit_error(error),
    Err(other) => report_runtime_error(other),
}
```

Do not use formatted error strings as state-machine input. Stable typed variants and
sources are the API contract.

## Re-exported lower-level surfaces

The facade re-exports canonical types from `vsh-types`, `vsh-vfs`, `vsh-policy`,
`vsh-monty`, `vsh-store`, and `vsh-commit`. Use them to configure the facade or process
receipts. Constructing a parallel commit path from lower-level parts defeats the
single-writer architecture and is not recommended.
