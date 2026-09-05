# Rust SDK

`vsh` is the application-facing native crate. It re-exports the implementation from
`vsh-runtime`, which is also called by Python through PyO3.

## Add the crate

Keep the exact version pin required by the project supply-chain policy:

```toml
[dependencies]
vsh = "=0.5.0"
```

The mirror crate `vbash = "=0.5.0"` contains no implementation and re-exports
the exact matching `vsh` release under the same `use vsh::...` library name. New
applications should depend on `vsh` directly.

The default production configuration expects a matching trusted
`vsh-monty-worker` executable. Put it on `PATH` or set an explicit path.

## Open and preview

```rust
use vsh::{ReceiptDetail, RunRequest, Runtime, RuntimeConfig};

fn main() -> Result<(), vsh::VshError> {
    let config = RuntimeConfig::new("project")
        .with_worker_path("/opt/vsh/bin/vsh-monty-worker");
    let runtime = Runtime::open(config)?;

    let receipt = runtime.preview(
        RunRequest::new(
            "from pathlib import Path\n\
             text = Path('/workspace/input.txt').read_text()\n\
             Path('/workspace/output.txt').write_text(text.upper())\n\
             len(text)",
        )
        .with_intent("Create reviewed uppercase output")
        .with_detail(ReceiptDetail::Full),
    )?;

    println!("transaction: {}", receipt.transaction);
    println!("decision: {:?}", receipt.decision);
    println!("changes: {:?}", receipt.changes);
    Ok(())
}
```

## Facade design

The facade exports the high-level runtime plus stable types from the internal crates.
Most applications should begin with:

- `Runtime` and `RuntimeConfig`;
- `RunRequest`, `RunMode`, and `ReceiptDetail`;
- `Receipt`, `RuntimeDecision`, and `StageTimings`;
- `ExecutionBudget`;
- `VshError`;
- identity and state values such as `TransactionId`, `TransactionState`, and `VPath`.

Lower-level commit, policy, store, VFS, and Monty types are re-exported for hosts that
need custom deterministic policy or operational reporting without learning internal
module paths.

## Execution choices

| Configuration | Isolation | Intended use |
|---|---|---|
| Default supervised worker | Process crash boundary and worker heap limit | Production and untrusted programs |
| `with_worker_path(path)` | Same, explicit binary identity | Packaged services and hermetic deployments |
| `with_in_process_execution()` | No crash isolation; process-local harness | Trusted tests and benchmarks only |

!!! danger "In-process execution is not a production optimization"

    It disables the worker crash boundary and per-worker heap enforcement. Never use it
    for hostile, model-authored, or otherwise unreviewed source.

## Concurrency

One `Runtime` has no process-global execution lock. Worker pooling is configured per
runtime, and separate workspace runtimes can execute concurrently. Same-workspace
commit serialization covers only the trusted revalidation/mutation window.

Continue with the [Rust API reference](api.md), [crate map](crates.md), or
[complete examples](examples.md).
