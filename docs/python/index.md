# Python SDK

Install `vsh-python`, import `vsh`. The Python layer is a PyO3 binding to the same Rust
runtime used by native applications. It does not simulate files in Python or fall back
to a second implementation.

```bash
python -m pip install vsh-python==0.5.0
```

CPython 3.11–3.14 wheels bundle a matching supervised worker. MCP is optional:
`vsh-python[mcp]==0.5.0`. The metadata-only `vbash` mirror installer depends on
the matching primary distribution and adds no import package.

Pydantic AI support is optional too: install `vsh-python[pydantic-ai]` and attach
`VshCapability` through `Agent(capabilities=[...])`. See the
[native Pydantic AI integration](../integrations/pydantic-ai.md). Application-owned
evidence review is covered by [commit hooks](hooks.md); the optional
[LLM commit judge](commit-judge.md) can approve pending transactions directly.
Build one end to end with the [deterministic review](../tutorials/pydantic-ai-deterministic.md)
or [LLM judge](../tutorials/pydantic-ai-judge.md) tutorial.

The `vsh_*` guest functions are included in VSH; see
[installation and worker setup](../start/index.md).

## Open a runtime once

```python
from vsh import Runtime

runtime = Runtime.open("./project", policy="balanced")
```

`./project` must already exist. It appears to the guest as `/workspace`. The default
protected data directory is `.vsh-runtime/data`. For a managed deployment, the trusted
host can select an external data directory and exact worker binary:

```python
runtime = Runtime.open(
    "/srv/workspaces/project-17",
    data_directory="/srv/vsh-state/project-17",
    worker_path="/opt/vsh/bin/vsh-monty-worker",
    policy="strict",
)
```

These paths and policy are application configuration, not model-controlled arguments.
Unsafe workspace/data overlap and redirected roots are rejected. Open also performs
startup recovery. Reuse the instance, but remember that each run creates a new snapshot.

## Choose the right call

| Call | Purpose |
|---|---|
| `preview(source, intent=..., detail=..., budget=...)` | Concise preview of one program |
| `preview(RunRequest(...))` | Preview a prepared immutable request; overrides its mode |
| `run(RunRequest(..., mode=RunMode.AUTO))` | Execute and commit only deterministic auto-approval |
| `commit(transaction, now_ms)` | Promote the exact retained or approved artifact |
| `approve(transaction, principal, issued, expires)` | Trusted approval of pending work |
| `discard_preview(transaction)` | Release an auto-approved process-local artifact |
| `recover()` | Inspect and process bounded recovery work |

Do not pass request options twice: `preview(request_object, detail=...)` is an error.
Configure that request before passing it. The source argument is named `request` in
the Python signature; the convenient positional form avoids that distinction.

## Inspect typed results and evidence

```python
from vsh import ReceiptDetail

preview = runtime.preview("{'answer': 6 * 7}", detail=ReceiptDetail.COMPACT)
assert preview.result == {"answer": 42}
print(preview.timings_ns())
runtime.discard_preview(preview.transaction)
```

Results cross through Monty's typed converter, not JSON. A valid result can accompany
a denied or pending transaction. Check `decision` and `state`; only `committed` confirms
verified application. `changes` contains `(path, kind)` entries in full detail, not
before/after text. Return a bounded review bundle when the caller needs content.

## Concurrency and memory

Native open, run, preview, commit and recovery release the GIL while Rust works.
This enables useful concurrency, not unlimited capacity. Apply admission control in
your application; same-workspace commits serialize and concurrent edits may become stale.

Guest heap, native result and pending-cache limits protect different allocations.
The default pending cache retains at most 64 auto-approved artifacts or 128 MiB of
encoded artifacts. Finish every preview lifecycle, including read-only requests.
See [efficient usage](../guides/efficient-usage.md).

## Handle errors by category

Native failures derive from `VshRuntimeError`: execution, state, stale-input, recovery
and internal errors have dedicated subclasses. Bad Python argument combinations may
raise `TypeError` or `ValueError`. Avoid parsing error strings to control the lifecycle.

Start with the [complete first transaction](../start/index.md), then run the
[cookbook](examples.md). The [API reference](api.md) lists every exported SDK class,
property and method.
