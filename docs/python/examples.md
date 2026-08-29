# Python examples

These examples use the public PyO3 surface and one existing workspace. Guest paths are
rooted at `/workspace`; host paths never enter the Monty program.

## Preview a multi-file edit

```python
from pathlib import Path

from vsh import ReceiptDetail, Runtime

root = Path("project").resolve(strict=True)
runtime = Runtime.open(root)
receipt = runtime.preview(
    """
from pathlib import Path

source = Path('/workspace/src/name.txt').read_text().strip()
Path('/workspace/generated').mkdir(parents=True, exist_ok=True)
Path('/workspace/generated/name.txt').write_text(source.upper() + '\n')
Path('/workspace/generated/meta.txt').write_text('generated-by=vsh\n')
{'source': source, 'files': 2}
""",
    intent="Regenerate checked-in name assets",
    detail=ReceiptDetail.FULL,
)

assert not (root / "generated" / "name.txt").exists()
for path, kind in receipt.changes:
    print(kind, path)
print(receipt.result)
```

## Promote an auto-approved preview

```python
import time

if receipt.decision == "auto_approved":
    committed = runtime.commit(
        receipt.transaction,
        time.time_ns() // 1_000_000,
    )
    assert committed.state == "committed"
    assert committed.committed
else:
    print("Review required:", receipt.risk_flags)
```

Do not execute the source again after review. The returned transaction is the exact
artifact that was inspected.

## Strict review gate

```python
import time

from vsh import Runtime

runtime = Runtime.open("project", policy="strict")
preview = runtime.preview(
    "from pathlib import Path\n"
    "Path('/workspace/reviewed.txt').write_text('yes')",
    intent="Apply approved change CHG-1842",
)
assert preview.decision == "pending_approval"

now = time.time_ns() // 1_000_000
runtime.approve(
    preview.transaction,
    principal="reviewer:alice/change:1842",
    issued_at_unix_ms=now,
    expires_at_unix_ms=now + 30_000,
)
committed = runtime.commit(preview.transaction, now + 1)
```

The trusted application—not Monty—chooses the principal and time window.

## Bound an agent-generated program

```python
from vsh import ExecutionBudget, RunRequest

request = RunRequest(
    agent_generated_source,
    intent="Update project metadata only",
    budget=ExecutionBudget(
        max_program_bytes=64 * 1024,
        max_duration_ms=300,
        max_memory_bytes=64 * 1024 * 1024,
        max_os_calls=1_500,
        max_read_bytes=8 * 1024 * 1024,
        max_write_bytes=2 * 1024 * 1024,
        max_output_bytes=32 * 1024,
        max_result_bytes=64 * 1024,
    ),
)
preview = runtime.preview(request)
```

## Handle stale input

```python
import time

from vsh import VshStaleError

try:
    runtime.commit(preview.transaction, time.time_ns() // 1_000_000)
except VshStaleError as error:
    # Re-preview against current state and ask for review again.
    print(f"preview is stale: {error}")
```

Never catch stale state and apply the old output yourself. That bypasses the property
the transaction boundary exists to enforce.

## Use auto mode for bounded jobs

```python
from vsh import RunMode, RunRequest

receipt = runtime.run(
    RunRequest(
        "from pathlib import Path\n"
        "Path('/workspace/status.txt').write_text('ready\n')",
        intent="Refresh local status marker",
        mode=RunMode.AUTO,
    )
)

if not receipt.committed:
    print(receipt.decision, receipt.risk_flags, receipt.deny_reason)
```

Auto mode does not override policy. Escalated or denied work remains non-mutating.

## Recover at service startup

```python
from vsh import Runtime, VshRecoveryError

try:
    runtime = Runtime.open("project")  # startup recovery runs here
    report = runtime.recover()
except VshRecoveryError as error:
    raise SystemExit(f"operator review required: {error}") from error

if report.conflicts or report.orphaned:
    raise SystemExit(f"unresolved recovery state: {report.conflicts}")
```

## Threaded hosts

Native calls release the GIL, so separate runtime roots may execute from Python worker
threads. Do not share a preview transaction across processes: process-local
auto-approved artifacts belong to the runtime that created them.
