# Start with VSH

VSH has one execution model and three convenient entry points. Begin with `preview`:
it executes the whole program and returns evidence, but it never changes host files.

## Prerequisites

- Python SDK and MCP: CPython 3.11–3.14.
- Rust SDK: Rust 1.95.0, as pinned by `rust-toolchain.toml`.
- A real workspace directory that the VSH process may read and, for commit, write.
- Production Rust embedding: the exact `vsh-monty-worker` executable. Python wheels
  bundle and resolve the worker automatically.

## Install

=== "Python"

    The distribution is named `vsh-python`; the import package and CLI are named `vsh`.

    ```bash
    uv add vsh-python==0.3.1
    ```

    ```bash
    python -m pip install vsh-python==0.3.1
    ```

=== "MCP"

    ```bash
    uv add 'vsh-python[mcp]==0.3.1'
    uv run vsh serve
    ```

=== "Rust"

    ```toml
    [dependencies]
    vsh = "=0.3.1"
    ```

    `vsh-runtime` remains the implementation crate; applications use the `vsh` facade.

!!! note "Legacy installer names"

    `vbash==0.3.1` on PyPI installs exactly `vsh-python==0.3.1` and contains no
    modules. The crates.io `vbash = "=0.3.1"` crate similarly re-exports exactly
    `vsh = "=0.3.1"`. New dependency declarations should use the primary names above.

## First preview

Create a workspace with `input.txt`, then run:

=== "Python"

    ```python
    from pathlib import Path

    from vsh import ReceiptDetail, Runtime

    workspace = Path("demo-workspace").resolve()
    runtime = Runtime.open(workspace)
    receipt = runtime.preview(
        """
    from pathlib import Path
    value = Path('/workspace/input.txt').read_text()
    Path('/workspace/output.txt').write_text(value.upper())
    len(value)
    """,
        intent="Create an uppercase derivative",
        detail=ReceiptDetail.FULL,
    )

    print(receipt.state)       # auto_approved or pending_approval
    print(receipt.result)      # typed Python value
    print(receipt.changes)     # [('output.txt', 'create')]
    assert not (workspace / "output.txt").exists()
    ```

=== "CLI"

    ```bash
    vsh run \
      --workspace demo-workspace \
      --mode preview \
      --detail full \
      --intent 'Create an uppercase derivative' \
      --code "from pathlib import Path; value = Path('/workspace/input.txt').read_text(); Path('/workspace/output.txt').write_text(value.upper()); len(value)"
    ```

=== "Rust"

    ```rust
    use vsh::{ReceiptDetail, RunRequest, Runtime, RuntimeConfig};

    fn main() -> Result<(), vsh::VshError> {
        let runtime = Runtime::open(RuntimeConfig::new("demo-workspace"))?;
        let receipt = runtime.preview(
            RunRequest::new(
                "from pathlib import Path\n\
                 value = Path('/workspace/input.txt').read_text()\n\
                 Path('/workspace/output.txt').write_text(value.upper())\n\
                 len(value)",
            )
            .with_intent("Create an uppercase derivative")
            .with_detail(ReceiptDetail::Full),
        )?;

        println!("transaction: {}", receipt.transaction);
        println!("decision: {:?}", receipt.decision);
        println!("changes: {:?}", receipt.changes);
        Ok(())
    }
    ```

## Read a receipt before commit

Check these fields in order:

1. `decision`: denied, auto-approved, or pending independent approval.
2. `changed_paths` and `changes`: the bounded canonical change set.
3. `risk_flags` in Python or the `RuntimeDecision` manifest in Rust.
4. `result` / `value` and captured `stdout`.
5. execution counters and per-stage timings.
6. `transaction`: the exact handle to discard, approve, or promote.

If an auto-approved preview is acceptable, promote the exact transaction rather than
rerunning its source:

```python
import time

committed = runtime.commit(receipt.transaction, time.time_ns() // 1_000_000)
assert committed.committed
```

Promotion revalidates the dependencies captured by the preview. If an input changed,
commit fails stale before applying the virtual output.

## Next

- Learn the [pipeline and trust boundaries](how-it-works.md).
- Choose a [transaction workflow](../guides/transactions.md).
- Configure the [Python SDK](../python/) or [Rust SDK](../rust/).
- Connect a coding agent through [MCP](../integrations/mcp.md).
