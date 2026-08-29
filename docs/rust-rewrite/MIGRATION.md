# Migration from the Python command engine to VSH 0.3 native runtime

The PyPI distribution remains `vbash` and the import package remains `vsh`, but VSH
0.3 replaces the command-specific Python simulator/executor lifecycle with one native
transaction API. The built wheel contains no Python fallback engine.

## Python API

Old command models such as `LsCommand`, registry lookup, `snapshot_workspace`,
`simulate_command`, `approve_plan`, and `execute_approved` are not compatibility
aliases. Compose one Monty program and call the PyO3-backed runtime:

```python
from vsh import RunMode, RunRequest, Runtime

runtime = Runtime.open("/path/to/workspace")
receipt = runtime.run(
    RunRequest(
        "from pathlib import Path\n"
        "Path('/workspace/out.txt').write_text('native')",
        mode=RunMode.AUTO,
    )
)
```

Use `runtime.preview(request)` when no host mutation is allowed. An auto-approved
preview may be promoted with `runtime.commit(receipt.transaction, now_unix_ms)` in the
same live runtime. Under strict policy, `approve(...)` binds an independent principal
and expiry to the exact durable artifact before commit.
Call `runtime.discard_preview(receipt.transaction)` when an auto-approved preview will
not be promoted; this releases its bounded process-local retention without host effect.

## MCP and CLI

The former search/schema/snapshot/simulate/approve/execute tool sequence is replaced by
one normal MCP tool, `vsh_run`. Put the complete multi-file operation in one program.
Preview promotion is a second `vsh_run` call containing the returned `transaction`, no
code, and `mode="auto"`.

The CLI equivalents are:

```bash
vsh run --workspace . --mode preview --code 'from pathlib import Path; Path("/workspace/a").write_text("x")'
vsh run --workspace . --mode auto --transaction TRANSACTION_ID
vsh serve
```

## Rust API

Depend on package `vsh-runtime` at the exact release version; its library target is
named `vsh`. Native deployments also install the matching `vsh-monty-worker` or pass a
trusted executable path with `RuntimeConfig::with_worker_path`.

## Deliberate compatibility breaks

- There is no shell-command registry or per-command schema in the release wheel.
- There is no JSON hop between simulation, policy, and commit.
- Results are native Rust/Python Monty values; unsupported Python projections fail
  before an auto-commit.
- Protected paths are hidden and denied before their names or bytes enter Monty.
- Preview handles are bounded; auto-approved handles are process-local, while
  independent-approval artifacts are durable.
