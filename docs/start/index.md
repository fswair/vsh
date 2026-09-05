# Start with a transaction

VSH runs a bounded Python program against a virtual copy of a workspace. You inspect
what it would change, then commit that exact result. Python and Rust use the same
native engine; neither needs a second simulator.

## Choose your installation

| Host | Package | Entry point |
|---|---|---|
| Python 3.11–3.14 | `vsh-python` | `from vsh import Runtime` |
| Rust | `vsh` | `use vsh::Runtime` |
| MCP client | `vsh-python[mcp]` | `vsh serve` |

```bash
python -m pip install vsh-python==0.5.0
```

```toml
[dependencies]
vsh = "=0.5.0"
```

Python wheels bundle the worker. Rust applications must deploy a matching
`vsh-monty-worker` executable; see [Rust setup](../rust/index.md). The `vbash`
packages are metadata-only mirrors, not a different engine. Prefer the primary names.

!!! note "VSH 0.5.0 surface"

    The Monty 0.0.22 integration, ten in-sandbox `vsh_*` functions and the September 5
    optimizations are part of VSH 0.5.0. The Python wheel bundles the matching worker;
    native Rust deployments must supply it as described below. Contributors can use
    the [source build](../development.md).

## A complete first run

Save this as `first_transaction.py` and run it with Python. It owns a temporary
workspace, so it cannot overwrite files in your project.

```python
import time
from pathlib import Path
from tempfile import TemporaryDirectory

from vsh import ReceiptDetail, Runtime

with TemporaryDirectory(prefix="vsh-first-") as directory:
    workspace = Path(directory)
    (workspace / "input.txt").write_text("hello\n", encoding="utf-8")
    runtime = Runtime.open(workspace)
    preview = runtime.preview(
        """
from pathlib import Path
before = Path('/workspace/input.txt').read_text()
after = before.upper()
Path('/workspace/output.txt').write_text(after)
{'before': before, 'after': after}
""",
        intent="Create an uppercase derivative of the fixture",
        detail=ReceiptDetail.FULL,
    )

    assert not (workspace / "output.txt").exists()
    assert preview.decision == "auto_approved"
    assert preview.changes == [("output.txt", "create")]
    assert preview.result == {"before": "hello\n", "after": "HELLO\n"}

    # This fixture's exact expected content is our trusted review.
    committed = runtime.commit(preview.transaction, time.time_ns() // 1_000_000)
    assert committed.committed
    assert (workspace / "output.txt").read_text() == "HELLO\n"
    print(committed.state, committed.changed_paths)  # committed 1
```

`/workspace` is a synthetic guest namespace, not your host's absolute directory.
Preview stages user-file changes only in the virtual filesystem. Runtime storage,
captured blobs and recovery metadata can still be written to disk.

## What to inspect

Start with `decision`, then the changed paths and their intended content. A successful
program is not necessarily approved; an approved preview is not yet committed.
`receipt.diff` is an identity digest, **not a text diff**. Full detail supplies path/kind
entries in Python. The small result above explicitly returns content for review.

Commit the returned transaction ID using the same live runtime. Do not rerun the
program and assume the output is identical. If an observed input changes, commit
rejects stale work before applying its proposed changes.

If you only wanted analysis, release its auto-approved preview with
`runtime.discard_preview(preview.transaction)`. Even read-only previews occupy the
bounded pending cache.

## Continue by task

- [Understand snapshots and execution](how-it-works.md).
- [Choose an appropriate use case](use-cases.md).
- [Python cookbook](../python/examples.md): migration, generation, review and stale input.
- [Rust cookbook](../rust/examples.md): run the same guest program natively.
- [MCP](../integrations/mcp.md): expose one transaction tool to an agent.
- [Pydantic AI, deterministic review](../tutorials/pydantic-ai-deterministic.md): attach
  a local evidence rule to the native filesystem capability.
- [Pydantic AI, LLM judge](../tutorials/pydantic-ai-judge.md): review canonical changes
  and exact content with a separately configured model.
- [Efficient usage](../guides/efficient-usage.md): keep latency, memory and context costs bounded.
