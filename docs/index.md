---
hide:
  - toc
---

<section class="vsh-hero">
  <span class="vsh-kicker">Transactional filesystem simulation</span>
  <h1>Low-latency simulation for Rust and Python.</h1>
  <p class="vsh-lead">
    VSH runs untrusted workspace automation against an immutable snapshot, produces a
    canonical diff, applies deterministic policy, and commits only after revalidation.
    One Rust core powers native Rust, Python, MCP, and coding-agent workflows.
  </p>
  <div class="vsh-actions">
    <a class="vsh-button vsh-button--primary" href="start/">Get started</a>
    <a class="vsh-button" href="start/how-it-works/">How it works</a>
  </div>
</section>

## What VSH solves

Agent-written filesystem code is useful precisely because it can change many files.
That same power makes a wrong path, stale read, replayed approval, or partial crash
expensive. A conventional dry run predicts; VSH executes the real program against
virtual state and binds the resulting artifact to what was actually observed.

## Validated locally

- **1.3–16.5 µs:** estimated incremental PyO3 p50 in post-hardening cases.
- **4.15× / 4.20×:** four-runtime native / Python throughput speedup.
- **134 + 38 tests:** Rust and shipped-Python tests in the merge record.

## One semantic core, two SDKs

Python does not reimplement simulation or commit semantics. The `vbash` wheel exposes
the native runtime through PyO3 and bundles the matching supervised Monty worker. Rust
embedders use the `vsh-runtime` crate directly. Both produce the same transaction,
snapshot, diff, decision, state transition, and commit proof.

=== "Python"

    ```python
    from vsh import ReceiptDetail, RunMode, RunRequest, Runtime

    runtime = Runtime.open("/workspace/project")
    receipt = runtime.run(
        RunRequest(
            """
    from pathlib import Path
    source = Path('/workspace/input.txt').read_text()
    Path('/workspace/output.txt').write_text(source.upper())
    {'bytes': len(source)}
    """,
            mode=RunMode.PREVIEW,
            detail=ReceiptDetail.FULL,
        )
    )
    print(receipt.decision, receipt.changes)
    ```

=== "Rust"

    ```rust
    use vsh::{ReceiptDetail, RunRequest, Runtime, RuntimeConfig};

    # fn main() -> Result<(), vsh::VshError> {
    let runtime = Runtime::open(RuntimeConfig::new("/workspace/project"))?;
    let receipt = runtime.preview(
        RunRequest::new(
            "from pathlib import Path\n\
             Path('/workspace/output.txt').write_text('ready')",
        )
        .with_detail(ReceiptDetail::Full),
    )?;
    println!("{} {:?}", receipt.transaction, receipt.decision);
    # Ok(())
    # }
    ```

## Why it is different

- **Simulation is executable state.** Reads and writes occur through a typed `VirtualFs`,
  not a shell-string heuristic.
- **Approval is exact.** Program, intent, snapshot, dependencies, policy, configuration,
  and canonical diff contribute to transaction identity.
- **Commit distrusts elapsed time.** Dependencies and capability roots are revalidated
  immediately before mutation.
- **Failure is recoverable.** Durable intent, journals, markers, and verification make
  interrupted commits resolvable without guessing ownership.
- **The boundary stays thin.** Python crosses PyO3 once per transaction and receives
  native typed values rather than a JSON round trip.

## Choose an entry point

| You are building | Start with | Why |
|---|---|---|
| A Python automation service | [Python SDK](python/) | Native safety with Python ergonomics and typed exceptions |
| A Rust host or agent runtime | [Rust SDK](rust/) | Direct access to configuration, policy, storage, and receipts |
| A local coding-agent integration | [MCP server](integrations/mcp/) | One compact `vsh_run` tool over stdio |
| A governed agent workflow | [Agent environments](integrations/agents/) | Preview-first protocol, bounded output, exact promotion |
| A performance-sensitive deployment | [Benchmarks](performance/) | Reproducible native/PyO3 latency, scaling, and RSS records |

!!! note "What VSH is not"

    VSH is not a POSIX shell, container, general Python sandbox, or network sandbox.
    Monty receives no host mount, subprocess capability, network capability, or ambient
    environment. Use VSH for bounded workspace transformations whose filesystem effects
    must be inspected, approved, and committed safely.
