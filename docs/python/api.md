# Python API reference

Import these names from `vsh`, supplied by the `vsh-python` distribution. This reference
describes the native boundary: values are typed Python projections, requests are
immutable, and all execution/commit semantics live in Rust. For complete programs,
start with the [cookbook](examples.md), not isolated API fragments.

The shipped `vsh._native.pyi` is the static signature contract. Guest `vsh_*` functions
are a separate [Monty program surface](../integrations/monty-tools.md), not module exports.

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

The Python distribution, extension, Rust workspace, and worker share version `0.5.0`.

This is the package version exported by `vsh`; development changes can share that
version string, so record the checkout revision for reproducible evidence.

## `RunMode`

| Member | Behavior |
|---|---|
| `RunMode.PREVIEW` | Execute, diff, and decide without applying user-workspace changes |
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
`max_os_calls` counts both `pathlib`/typed OS suspensions and high-level VSH function
calls. The functions available in every `code` program are documented in
[VSH functions inside Monty](../integrations/monty-tools.md).

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
    hook_id: str | None = ...,
    hook_scope: HookScope | None = ...,
    review_content_bytes: int = 0,
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
    request: str,
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

Both forms cross PyO3 exactly once and force preview semantics. Each creates a fresh
snapshot and overlay. `mode` is intentionally
absent from the source-code overload because it never applies the program's proposed
user-file changes.
When the first argument is a `RunRequest`, configuration must remain on that request;
passing `intent`, `detail`, or `budget` again raises `TypeError` instead of merging two
sources of truth.

### `discard_preview(transaction: str) -> bool`

Removes one process-local auto-approved preview. Returns `False` when no matching
preview exists.

Use this for abandoned mutations and completed read-only analysis. The default cache
fails closed at 64 entries or 128 MiB encoded artifact bytes and rejects duplicate
exact pending identities. This method is not a general durable-artifact cancellation
or blob-store garbage-collection API. Auto-approved handles belong to the same live
runtime; approval-required artifacts are durable.

### `approve(...) -> str`

```text
runtime.approve(
    transaction: str,
    principal: str,
    issued_at_unix_ms: int,
    expires_at_unix_ms: int,
) -> str
```

Creates an exact approval grant for a pending transaction. The caller must authenticate
and authorize the reviewer; the principal string itself is not authentication.
Returns the new lifecycle
state (`"approved"`). The validity interval must be coherent and commit time must fall
inside it.

### `commit(transaction: str, now_unix_ms: int) -> Receipt`

Consumes a single-use reservation, revalidates bound dependencies and capability
identity, applies the canonical plan through the trusted committer, verifies the host,
and returns a committed receipt. Stale input raises `VshStaleError` before mutation.

### `recover() -> RecoveryReport`

Reruns bounded recovery and returns what was finalized, rolled back, cleaned, left
orphaned, or reported as conflict.

The evidence-first handler surface (`HookedRuntime`, `HookScope`, `RequestEvent`,
`HookDecision`, `CommitPreparation`, and `CommitResolution`) is documented on the
[commit hooks](hooks.md) page. The optional agent-native surface is documented under
[Pydantic AI capability](../integrations/pydantic-ai.md). `CommitJudge` and `JudgeReport`
are documented under [LLM commit judge](commit-judge.md).

## Optional Pydantic AI surface

Import these classes from `vsh.pydantic_ai` after installing the `pydantic-ai` extra:

```python
from vsh.pydantic_ai import CommitJudge, JudgeReport, VshCapability, VshToolResult
```

`VshCapability(workspace, ...)` owns a native runtime and contributes ten filesystem
tools plus atomic `vsh_run` to `Agent(capabilities=[...])`. It accepts native runtime
configuration together with `hook_handler`, `hook_scope`, `hook_id`,
`review_content_bytes`, capability `id`, and `defer_loading`. Construct it directly;
there is no `VshCapability.open` alias.

`VshToolResult` contains `transaction`, `state`, JSON-compatible `result`,
`changed_paths`, optional `hook_verdict`, optional `feedback`, and the derived
`requires_review` property. Pending, rejected, and denied outcomes withhold the guest
result from the calling agent.

`CommitJudge(model, ...)` builds a bounded structured reviewer. Configure its additive
`review_instructions`, model settings, content allowlist, usage limits, provider output
cap, timeout, input-byte cap, and concurrency bound. Pass `judge.hook_handler` to the
capability or `HookedRuntime`; the judge object itself is not callable. `JudgeReport`
contains `decision`, `reason`, evidence references, concerns, and missing evidence.

See the [capability constructor and tool reference](../integrations/pydantic-ai.md),
[judge constructor and evidence contract](commit-judge.md), and guided
[deterministic](../tutorials/pydantic-ai-deterministic.md) and
[judge](../tutorials/pydantic-ai-judge.md) applications.

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
| `deny_reason` | `str \| None` | Stable denial description |

### Output and effects

| Property | Type | Meaning |
|---|---|---|
| `changed_paths` | `int` | Canonical change count |
| `changes` | `list[tuple[str, str]]` | Full path/kind list when requested |
| `result` | `object` | Native Python projection of Monty's returned value |
| `result_repr` | `str` | Full `repr()` of the projected value, constructed on access |
| `stdout` | `str` | Bounded captured `print()` output |

Change kinds are `create`, `delete`, `modify`, and `metadata_change`. Rename effects are
represented by the canonical before/after entries produced by the native diff.

`diff` is a digest and `changes` does not contain text. Full detail does not produce
a unified diff. Return bounded content evidence from the guest when review needs it.
`result_repr` has no independent native truncation; avoid it for structured processing
or large results. MCP/CLI apply their own text caps after this representation is made.
The decision remains the original policy decision even when state becomes `committed`.

### Resource counters

`os_calls`, `read_bytes`, `write_bytes`, `directory_entries`, `output_bytes`,
`denied_accesses`, and `result_bytes` describe work performed by the worker adapter.

Read/write counts describe cumulative work, not just final diff size. `result_bytes`
tracks Monty's bounded host-footprint estimate, not network JSON bytes or token count.

### Commit evidence

| Property | Type | Meaning |
|---|---|---|
| `committed` | `bool` | Whether host effects were applied and verified |
| `commit_operations` | `int \| None` | Number of trusted commit operations |
| `verified_paths` | `int \| None` | Paths verified after application |
| `cleanup_pending` | `bool` | Durable cleanup remains for recovery |

### Timings

`timings_ns() -> Mapping[str, int]` returns `snapshot`, `execute`, `diff`, `policy`,
`bind_and_store`, `commit`, and `total`. They are monotonic stage measurements intended
for profiling and receipts, not an authorization signal.

For a separate `commit()`, `total` is the retained preview total plus the measured
committer interval, not current API-call latency or reviewer wait time. The `commit`
interval excludes preceding artifact persistence, plan construction and reservation.
Measure outer wall time separately when you need complete promotion latency.

## `RecoveryReport`

| Property | Meaning |
|---|---|
| `finalized_commits` | Commit was already applied and recovery finalized its state |
| `rolled_back` | Partial work was safely returned to the original state |
| `cleaned` | Obsolete trusted artifacts were removed |
| `orphaned` | Ownership could not be proven; item was left untouched |
| `conflicts` | `(transaction, path \| None, reason)` operator records |

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

Invalid Python arguments can also raise `TypeError` or `ValueError`. An internal
error is not a guarantee of successful rollback after commit began; inspect recovery.
