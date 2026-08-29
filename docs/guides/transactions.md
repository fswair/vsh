# Transactions

A VSH transaction is an immutable claim: this program, intent, snapshot, dependency
set, runtime configuration, policy, and canonical diff belong together. Reusing only
the source code is not equivalent to promoting a preview.

## Lifecycle

| State | Meaning | Host changed? | Next action |
|---|---|---:|---|
| `denied` | Policy or protected access rejected the artifact | No | Fix source, scope, or policy input |
| `auto_approved` | Deterministic policy allows the exact artifact | No in preview | Commit or discard through the same live runtime |
| `pending_approval` | Independent approval is required | No | Trusted host approves, then commits |
| `approved` | An unexpired principal grant is bound | No | Commit before grant expiry |
| `reserved` | Commit ownership is consumed | Not necessarily | Runtime completes or recovery resolves |
| `committed` | Effects were applied and verified | Yes | Retain receipt as evidence |
| `failed` | A terminal failure consumed the attempt | No or recovered | Inspect error/recovery report; do not replay blindly |

## Preview and exact promotion

```python
from pathlib import Path
import time

from vsh import ReceiptDetail, RunRequest, Runtime, VshStaleError

workspace = Path("project").resolve()
runtime = Runtime.open(workspace)
preview = runtime.preview(
    RunRequest(
        "from pathlib import Path\n"
        "text = Path('/workspace/a.txt').read_text()\n"
        "Path('/workspace/b.txt').write_text(text)\n"
        "len(text)",
        intent="Copy the reviewed input",
        detail=ReceiptDetail.FULL,
    )
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
must be promoted or discarded by the same live `Runtime` (or MCP server process).
Before commit, VSH persists the exact artifact and lifecycle record durably.

Transactions requiring independent approval are durable at preview completion and can
survive runtime restart.

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
