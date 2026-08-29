# Python SDK

The `vbash` distribution is a native Python package: its public `vsh` module is backed
by a PyO3 extension linked to the same Rust runtime used by native callers. There is no
Python simulator fallback.

## Install

```bash
uv add vbash==0.3.0
```

For the optional MCP adapter:

```bash
uv add 'vbash[mcp]==0.3.0'
```

Supported interpreters are CPython 3.11, 3.12, 3.13, and 3.14. Release wheels bundle
the exact `vsh-monty-worker` that matches the extension.

## Minimal transaction

```python
from pathlib import Path

from vsh import ReceiptDetail, RunMode, RunRequest, Runtime

workspace = Path("project").resolve(strict=True)
runtime = Runtime.open(workspace, policy="balanced")
receipt = runtime.run(
    RunRequest(
        """
from pathlib import Path
source = Path('/workspace/src/config.txt').read_text()
Path('/workspace/build/config.txt').write_text(source.strip() + '\n')
{'input_bytes': len(source)}
""",
        intent="Normalize generated configuration",
        mode=RunMode.PREVIEW,
        detail=ReceiptDetail.FULL,
    )
)

print(receipt.transaction)
print(receipt.decision)
print(receipt.result)       # native Python dict
print(receipt.changes)      # list[tuple[path, kind]]
```

## Boundary behavior

- `Runtime.open`, `run`, `preview`, `commit`, and `recover` release the GIL while native
  work is in progress.
- Monty values cross through the pinned typed converter, not JSON.
- Rust panics are contained before crossing FFI and become `VshInternalError`.
- All native failures derive from `VshRuntimeError`, so callers may catch broadly or by
  category.
- Each `run` call crosses Python/Rust once for the complete transaction; filesystem
  calls remain inside the native worker/runtime protocol.

## Runtime layout

```python
runtime = Runtime.open(
    workspace,
    data_directory="/trusted/vsh-data/project-17",
    policy="strict",
    worker_path="/opt/vsh/bin/vsh-monty-worker",
)
```

| Argument | Default | Meaning |
|---|---|---|
| `workspace` | required | Existing host directory exposed virtually as `/workspace` |
| `data_directory` | `.vsh-runtime/data` below workspace | Protected durable blobs, transaction state, and recovery artifacts |
| `policy` | `balanced` | `balanced`, `strict`, or `paranoid` deterministic policy |
| `worker_path` | bundled/resolved worker | Explicit trusted worker executable override |

An explicit data directory must not overlap the untrusted workspace through any
lexical, canonical, prospective, or symlink alias.

## Which method should I call?

| Method | Use it when |
|---|---|
| `run(request)` | Request mode should control preview vs deterministic auto-commit |
| `preview(request)` | Host mutation must be impossible in this call, regardless of request mode |
| `commit(transaction, now_ms)` | Promote one exact auto-approved or independently approved preview |
| `approve(...)` | A trusted principal accepts a pending transaction for a bounded window |
| `discard_preview(transaction)` | An unused process-local auto-approved preview should release capacity |
| `recover()` | The host needs an explicit recovery report after startup or an incident |

Continue with the complete [Python API reference](api.md) or tested
[workflow examples](examples.md).
