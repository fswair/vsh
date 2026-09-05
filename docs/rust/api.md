# Rust API reference

This page documents the application-facing `vsh` facade. Rustdoc remains the
exhaustive source for every re-exported lower-level field and error variant.

Use the [complete Rust cookbook](examples.md) for a compiled example. Unlike Python,
the Rust preview API accepts a borrowed `RunRequest`; it does not emulate Python's
source-string overload. The guest `vsh_*` surface is included in VSH.

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
| `with_commit_hook(hook)` | Bind a hook identity, scope, approval lifetime, and feedback bound |
| `workspace_root()` | Return the host workspace root |
| `data_directory()` | Return the trusted artifact root |
| `worker_path()` | Return the worker path or `None` in trusted in-process mode |
| `policy()` | Borrow the deterministic transaction policy |

Builder values contribute to runtime configuration identity where security relevant.
Idle-pool capacity is not an admission limit on concurrent calls. Reusing a runtime
retains clean workers and capabilities, but every execution captures a fresh snapshot.

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

Preview does not apply user-file changes; runtime metadata/storage can still be written.
Auto commits only deterministic `AutoApproved` work.
Compact omits per-path entries while retaining identity and counts; Full retains the
complete bounded canonical diff.

## `Runtime`

| Method | Result | Contract |
|---|---|---|
| `Runtime::open(config)` | `Runtime` | Establish capabilities, stores, worker supervision, and startup recovery |
| `startup_recovery()` | `&RecoveryReport` | Inspect recovery performed during open |
| `run(request)` | `Receipt` | Execute according to request mode |
| `preview(request)` | `Receipt` | Execute without applying proposed user-file changes |
| `discard_preview(id)` | `bool` | Release a process-local auto-approved artifact |
| `approve(id, principal, issued, expires)` | `TransactionRecord` | Bind an independent time-limited approval |
| `commit(id, now)` | `Receipt` | Single-use reserve, revalidate, apply, verify |
| `recover()` | `RecoveryReport` | Resolve bounded durable commit state |
| `transaction(id)` | `TransactionRecord` | Read current durable lifecycle state |
| `prepare_commit(id)` | `CommitPreparation` | Freeze an exact hook event without invoking host code |
| `resolve_commit(preparation, decision, now)` | `CommitResolution` | Revalidate and apply one typed hook decision |
| `fail_hook(preparation)` | `()` | Move an interrupted automatic approval into pending review |

All fallible methods return `Result<_, VshError>`.

Auto-approved preview IDs must be consumed by the same live runtime. The default
fail-closed retention caps are 64 artifacts and 128 MiB encoded bytes. Discard completed
analysis too. Pending approval artifacts are durable and can survive a compatible
runtime reopen; `approve` binds a `PrincipalId` but does not authenticate its owner.

## Commit hooks

`HookedRuntime<H: CommitHook>` is the synchronous Rust convenience owner. The default
`HookScope::ReviewRequired` invokes `H` only for deterministic policy escalations;
`HookScope::AllRequests` also includes successful read-only and auto-approved
simulations. Hard policy denial never invokes a hook.

Handlers receive an immutable `RequestEvent` containing the canonical diff, ordered
effects, risk evidence and all transaction-binding digests. They return
`HookDecision::{FollowPolicy, Approve, Review, Reject}`. `Review` uses the existing
`PendingApproval` state and carries feedback in `HookDecisionRecord`; there is no
separate review-only lifecycle state.

Async Rust hosts should call `prepare_commit`, await their own handler outside VSH
locks, then call `resolve_commit`. If handler execution fails, call `fail_hook` before
propagating the error. Direct `Runtime::commit` cannot bypass a configured applicable
hook.

`HookConfig::with_max_content_bytes(maximum)` opts into immutable content evidence;
the default is zero. When the hook is in scope, VSH captures eligible bounded before
content before final binding, then recomputes the canonical diff and policy. Native
content-read permissions also govern review capture and delivery.

`RequestEvent::contents` contains `ReviewContent { path, blob, bytes }` for canonical
before/after states and observed content reads. `content_complete` distinguishes full
content coverage from structural `evidence_complete`; oversized, stamped or protected
content must not be assumed reviewed. Bytes are loaded from hash-verified blobs, not
from current host paths during handler execution.

A valid `HookDecision::Approve` directly approves pending work and attempts its exact
commit. An additional human approval is not required. The Python
[LLM commit judge](../python/commit-judge.md) uses this same native protocol; Rust
applications can call their chosen model/service inside their own handler without
adding a provider dependency to the core.

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

`diff` is a digest. Each full `DiffEntry` includes `path`, `kind`, `before` and `after`
`NodeState` values, not text hunks. `value` is the typed `MontyObject`, whereas Python
projects it to a Python object. Native compatibility can represent values the pinned
Python converter cannot; choose `ResultCompatibility::Python` when that projection is
part of your downstream contract.

## `StageTimings`

All fields are nanoseconds measured with a monotonic clock:

| Field | Includes |
|---|---|
| `snapshot_ns` | Fresh capability-rooted metadata traversal and lazy snapshot construction |
| `execute_ns` | Monty, IPC, typed VirtualFs calls and their call-policy checks |
| `diff_ns` | Canonical diff freeze |
| `policy_ns` | Deterministic policy evaluation |
| `bind_and_store_ns` | Artifact binding and lifecycle writes |
| `commit_ns` | Committer revalidation, application and verification; excludes preceding persistence, plan creation and reservation |
| `total_ns` | Complete `run` elapsed time; separate promotion retains preview total plus measured commit interval |

Measure outer wall time for complete `commit()` API latency. Its receipt total does
not include reviewer wait time and is not a stopwatch for just that API invocation.

## `ExecutionBudget`

`ExecutionBudget` aliases `vsh_monty::ExecutionLimits` and has public fields for source
bytes, duration, recursion, memory, typed OS plus high-level VSH calls, read/write bytes,
per-call I/O, path bytes, directory entries, stdout, returned value, and exception
payload. Prefer struct update syntax from `Default` so later compatible fields receive
safe defaults. `vsh_monty::MONTY_VSH_TOOL_NAMES` exposes the stable in-sandbox function
names; see [VSH functions inside Monty](../integrations/monty-tools.md).

## `ArtifactLimits`

Trusted-host bounds for durable pending artifacts:

- total encoded artifact bytes;
- encoded result and retained stdout;
- canonical entries and dependencies;
- one-path bytes;
- process-local preview count and aggregate encoded bytes.

These do not replace execution budgets; they constrain the artifact VSH retains after
execution.

The guest duration is cumulative bytecode time; the heap cap applies to the supervised
worker. Neither is a total parent-process RSS or end-to-end request deadline. For all
defaults and measurement scope, see [policies and budgets](../guides/policies-and-budgets.md).

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
