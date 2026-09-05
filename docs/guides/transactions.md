# Transactions

A VSH transaction is an immutable claim: this program, intent, snapshot, dependency
set, runtime configuration, policy, and canonical diff belong together. Reusing only
the source code is not equivalent to promoting a preview.

The [cookbook](../python/examples.md) executes preview, restart, approval and stale
handling against disposable fixtures. Use it alongside this lifecycle reference.

## Lifecycle

| State | Meaning | Host changed? | Next action |
|---|---|---:|---|
| `denied` | Policy or protected access rejected the artifact | No | Fix source, scope, or policy input |
| `auto_approved` | Deterministic policy allows the exact artifact | No in preview | Commit or discard through the same live runtime |
| `pending_approval` | Independent approval is required | No | Trusted host approves, then commits |
| `approved` | An unexpired principal grant is bound | No | Commit before grant expiry |
| `stale` | Revalidation found changed dependencies | No proposed changes applied | Recompute and review a new transaction |
| `expired` | Approval window elapsed | No proposed changes applied | Inspect lifecycle; obtain a newly reviewed proposal |
| `reserved` | Commit ownership is consumed | Not necessarily | Runtime completes or recovery resolves |
| `committed` | Effects were applied and verified | Yes | Retain receipt as evidence |
| `recovery_required` | Commit was interrupted or verification needs recovery | Possibly partial | Stop and inspect recovery |
| `failed` | A terminal failure consumed the attempt | No or recovered | Inspect error/recovery report; do not replay blindly |

## Preview and exact promotion

```python
from pathlib import Path
import time

from vsh import ReceiptDetail, Runtime, VshStaleError

workspace = Path("project").resolve()
runtime = Runtime.open(workspace)
preview = runtime.preview(
    "from pathlib import Path\n"
    "text = Path('/workspace/a.txt').read_text()\n"
    "Path('/workspace/b.txt').write_text(text)\n"
    "len(text)",
    intent="Copy the reviewed input",
    detail=ReceiptDetail.FULL,
)

if preview.decision == "auto_approved":
    try:
        committed = runtime.commit(
            preview.transaction,
            time.time_ns() // 1_000_000,
        )
    except VshStaleError:
        # a.txt or another bound dependency changed after preview
        raise
```

Auto-approved previews use a bounded process-local cache for their exact artifact. They
must be promoted or discarded by the same live `Runtime`. An MCP process must also
retain that exact runtime in its bounded LRU; process lifetime alone is insufficient.
Before commit, VSH persists the exact artifact and lifecycle record durably.

Transactions requiring independent approval are durable at preview completion and can
survive runtime restart.

Preview isolation means proposed user-file effects are not applied. Opening a runtime
and executing previews can write trusted metadata and immutable blobs. A second preview
uses a fresh host snapshot, not the previous preview's overlay.

## Review content, not just counts

`diff` is a canonical digest. Python full detail includes `(path, kind)`; Rust full
detail includes before/after node-state identities and metadata. Neither SDK supplies
a unified text diff in the receipt. For bounded migrations, return selected before/after
content from the guest and compare it with the intended change and expected paths.

Treat source, stdout and returned content as untrusted review data. A model-generated
statement that a change is safe is not an independent approval. For richer review UIs,
build an explicitly trusted content renderer keyed to exact artifact evidence.

## Independent approval

Strict and paranoid mutation previews usually return `pending_approval`. The trusted
host supplies a stable principal and a validity window:

```python
now_ms = time.time_ns() // 1_000_000
state = runtime.approve(
    preview.transaction,
    principal="review-service:change-1842",
    issued_at_unix_ms=now_ms,
    expires_at_unix_ms=now_ms + 60_000,
)
assert state == "approved"
committed = runtime.commit(preview.transaction, now_ms + 1)
```

Approval does not replace policy. A denied transaction cannot be promoted, and an
approval for one transaction cannot authorize another diff or changed dependency set.
The SDK binds a principal label/digest and caller-provided times; your service must
authenticate and authorize the principal. Use a real trusted clock, not model-supplied
timestamps or the illustrative clock values used in unit tests.

## Auto mode

`RunMode.AUTO` is a convenience for transactions that deterministic policy
auto-approves. VSH executes, decides, reserves, revalidates, commits, and verifies in
one call. If policy denies or escalates, the call remains non-mutating.

Use auto mode for small, well-bounded, idempotent automation. Use explicit preview when
a person or agent must inspect exact changes.

## Discard

Discard an auto-approved process-local preview when it will not be committed:

```python
discarded = runtime.discard_preview(preview.transaction)
```

Retention is capped by both transaction count and encoded bytes. Failing to discard
unused handles may reach the configured capacity and fail closed; it cannot grow into
an unbounded parent-process cache.

Defaults are 64 auto-approved artifacts and 128 MiB encoded artifact bytes. Read-only
previews count too, and an identical still-pending transaction is rejected as a
duplicate. Discard releases process-local retention, not all stored blobs or durable
pending approvals. Raw MCP has no discard command; an SDK-owned service is preferable
for long-lived high-rate analysis.

## Process boundaries

One-shot CLI invocations exit after printing their receipt. A new process cannot load
the previous CLI's auto-approved preview. Use a live SDK runtime for exact promotion,
or strict pending artifacts with trusted approval for a durable review queue. See
[CLI workflows](cli.md) for the tested boundary.

## Recovery

`Runtime.open` runs startup recovery. Call `recover()` explicitly after an operational
incident or when surfacing a report to the host:

```python
report = runtime.recover()
print(report.finalized_commits, report.rolled_back, report.cleaned)
for transaction, path, reason in report.conflicts:
    print(transaction, path, reason)
```

An orphan or conflict means VSH could not prove safe ownership. It leaves ambiguous
data untouched for an operator rather than deleting or overwriting it.

## Concurrency

- Snapshot, simulation, diff, and policy may proceed independently.
- Only the short same-workspace revalidation/mutation window is serialized.
- Independent runtime roots can scale in parallel.
- A transaction is single-use; concurrent double commit cannot apply it twice.

Worker pooling limits idle reuse, not total incoming concurrency. Bound active requests
in the host. External editors do not participate in the runtime's commit lock; VSH's
revalidation and recovery contract is not universal multi-file atomic visibility.
