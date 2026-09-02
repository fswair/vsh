# Python API reference

All public names below are exported from `vsh`. Static signatures are shipped in
`vsh._native.pyi`; runtime behavior is implemented by the PyO3 extension.

## Module functions

### `engine_kind() -> str`

Returns `"rust"`. This is a diagnostic assertion that the native engine owns the
surface.

### `normalize_path(path: str) -> str`

Parses and canonicalizes a workspace-relative virtual path. It removes `.` segments,
resolves internal `..`, and rejects empty, escaping, absolute POSIX, and Windows-drive
paths.

```python
from vsh import normalize_path

assert normalize_path("src/vsh/./core/../lib.rs") == "src/vsh/lib.rs"
```

### `__version__: str`

The Python distribution, extension, Rust workspace, and worker share version `0.3.1`.

## `RunMode`

| Member | Behavior |
|---|---|
| `RunMode.PREVIEW` | Execute, diff, and decide without changing host files |
| `RunMode.AUTO` | Commit only if deterministic policy auto-approves; otherwise remain non-mutating |

## `ReceiptDetail`

| Member | Behavior |
|---|---|
| `ReceiptDetail.COMPACT` | Counts and digests; `changes` stays empty |
| `ReceiptDetail.FULL` | Includes the complete bounded canonical `(path, kind)` list |

## `ExecutionBudget`

```text
ExecutionBudget(
    *,
    max_program_bytes: int | None = ...,
    max_duration_ms: int | None = ...,
    max_recursion_depth: int | None = ...,
    max_memory_bytes: int | None = ...,
    max_os_calls: int | None = ...,
    max_read_bytes: int | None = ...,
    max_write_bytes: int | None = ...,
    max_io_call_bytes: int | None = ...,
    max_path_bytes: int | None = ...,
    max_directory_entries: int | None = ...,
    max_output_bytes: int | None = ...,
    max_result_bytes: int | None = ...,
    max_exception_bytes: int | None = ...,
)
```

Every argument is keyword-only. `None` selects the native default. Each value is
available as a read-only property with the same name. See
[Policies and budgets](../guides/policies-and-budgets.md) for defaults and tuning.

## `RunRequest`

```text
RunRequest(
    code: str,
    *,
    intent: str | None = ...,
    mode: RunMode | None = ...,
    detail: ReceiptDetail | None = ...,
    budget: ExecutionBudget | None = ...,
)
```

| Property | Meaning |
|---|---|
| `code` | Exact Monty source bound into transaction identity |
| `intent` | Optional trusted-host context bound independently from source |
| `mode` | Defaults to `PREVIEW` |
| `detail` | Defaults to `COMPACT` |
| `budget` | Independent resource ceilings for this execution |

The request object is immutable from Python after construction.

## `Runtime`

### `Runtime.open(...) -> Runtime`

```text
Runtime.open(
    workspace: str | os.PathLike[str],
    *,
    data_directory: str | os.PathLike[str] | None = ...,
    policy: str = "balanced",
    worker_path: str | os.PathLike[str] | None = ...,
) -> Runtime
```

Opens capability roots, validates separation, creates bounded stores, starts worker
supervision, and performs startup recovery. The workspace must already exist and be a
directory.

### `run(request: RunRequest) -> Receipt`

Executes according to `request.mode`. In auto mode, only an `auto_approved` decision
may proceed to commit.

### `preview(...) -> Receipt`

```text
preview(request: RunRequest) -> Receipt

preview(
    code: str,
    *,
    intent: str | None = ...,
    detail: ReceiptDetail | None = ...,
    budget: ExecutionBudget | None = ...,
) -> Receipt
```

The request overload preserves an immutable request that was prepared elsewhere. The
source-code overload constructs the same native request directly and is the compact
form for one-off previews:

```python
receipt = runtime.preview(
    "from pathlib import Path\nPath('/workspace/result.txt').write_text('ready')",
    intent="Create the reviewed result",
    detail=ReceiptDetail.FULL,
)
```

Both forms cross PyO3 exactly once and force preview semantics. `mode` is intentionally
absent from the source-code overload because this method can never mutate the host.
When the first argument is a `RunRequest`, configuration must remain on that request;
passing `intent`, `detail`, or `budget` again raises `TypeError` instead of merging two
sources of truth.

### `discard_preview(transaction: str) -> bool`

Removes one process-local auto-approved preview. Returns `False` when no matching
preview exists.

### `approve(...) -> str`

```text
runtime.approve(
    transaction: str,
    principal: str,
    issued_at_unix_ms: int,
    expires_at_unix_ms: int,
) -> str
```

Creates an exact approval grant for a pending transaction. Returns the new lifecycle
state (`"approved"`). The validity interval must be coherent and commit time must fall
inside it.

### `commit(transaction: str, now_unix_ms: int) -> Receipt`

Consumes a single-use reservation, revalidates bound dependencies and capability
identity, applies the canonical plan through the trusted committer, verifies the host,
and returns a committed receipt. Stale input raises `VshStaleError` before mutation.

### `recover() -> RecoveryReport`

Reruns bounded recovery and returns what was finalized, rolled back, cleaned, left
orphaned, or reported as conflict.

## `Receipt`

### Identity and decision

| Property | Type | Meaning |
|---|---|---|
| `transaction` | `str` | Exact transaction identifier |
| `base_snapshot` | `str` | Immutable input snapshot identifier |
| `state` | `str` | Current lifecycle state |
| `decision` | `str` | `denied`, `auto_approved`, or `pending_approval` |
| `diff` | `str` | Canonical diff digest |
| `risk_flags` | `list[str]` | Deterministic escalation reasons |
| `deny_reason` | `str | None` | Stable denial description |

### Output and effects

| Property | Type | Meaning |
|---|---|---|
| `changed_paths` | `int` | Canonical change count |
| `changes` | `list[tuple[str, str]]` | Full path/kind list when requested |
| `result` | `object` | Native Python projection of Monty's returned value |
| `result_repr` | `str` | Bounded diagnostic representation |
| `stdout` | `str` | Bounded captured `print()` output |

Change kinds are `create`, `delete`, `modify`, and `metadata_change`. Rename effects are
represented by the canonical before/after entries produced by the native diff.

### Resource counters

`os_calls`, `read_bytes`, `write_bytes`, `directory_entries`, `output_bytes`,
`denied_accesses`, and `result_bytes` describe work performed by the worker adapter.

### Commit evidence

| Property | Type | Meaning |
|---|---|---|
| `committed` | `bool` | Whether host effects were applied and verified |
| `commit_operations` | `int | None` | Number of trusted commit operations |
| `verified_paths` | `int | None` | Paths verified after application |
| `cleanup_pending` | `bool` | Durable cleanup remains for recovery |

### Timings

`timings_ns() -> Mapping[str, int]` returns `snapshot`, `execute`, `diff`, `policy`,
`bind_and_store`, `commit`, and `total`. They are monotonic stage measurements intended
for profiling and receipts, not an authorization signal.

## `RecoveryReport`

| Property | Meaning |
|---|---|
| `finalized_commits` | Commit was already applied and recovery finalized its state |
| `rolled_back` | Partial work was safely returned to the original state |
| `cleaned` | Obsolete trusted artifacts were removed |
| `orphaned` | Ownership could not be proven; item was left untouched |
| `conflicts` | `(transaction, path | None, reason)` operator records |

## Exceptions

```text
RuntimeError
└── VshRuntimeError
    ├── VshExecutionError
    ├── VshStateError
    ├── VshStaleError
    ├── VshRecoveryError
    └── VshInternalError
```

| Exception | Catch for |
|---|---|
| `VshExecutionError` | Monty compilation, execution, protocol, or hard-budget failure |
| `VshStateError` | Invalid lifecycle transition, approval, reservation, or replay |
| `VshStaleError` | Host dependencies changed after virtual execution |
| `VshRecoveryError` | Recovery conflict or inability to prove safe ownership |
| `VshInternalError` | Contained Rust panic or invariant failure at the native boundary |

Catch `VshRuntimeError` at application boundaries, then log the concrete subclass and
receipt/transaction context. Do not convert stale or recovery failures into blind
retries.
