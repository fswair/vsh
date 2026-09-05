# Python cookbook

These are complete native workflows, not snippets that rely on an unexplained
`project` directory. The source programs create temporary fixtures, assert isolation
and exact results, commit only reviewed fixture changes, and clean up their own data.
They need no model API key or network access.

The recipes use the `vsh_*` surface shipped in VSH 0.4.0. From a source checkout, build
the release extension and matching worker as described in [development](../development.md), then:

```bash
uv run --no-sync python examples/native/workflows.py
```

The [executable source](https://github.com/fswair/vsh/blob/main/examples/native/workflows.py)
contains all four workflows below. Each is exercised by the native runtime test suite.

## Bulk configuration migration

**Problem:** change timeouts consistently across services without overwriting a
concurrent edit or partially guessing the scope.

The fixture creates `services/billing/service.toml` and
`services/search/service.toml`, each containing `timeout = 5`. The guest program:

```python
limit = 20
files = vsh_glob('**/*.toml', path='/workspace/services', max_results=limit + 1)
assert 0 < len(files) <= limit, 'Split this migration into explicitly reviewed batches'
review = []
for file in files:
    before = vsh_read(file)
    assert before.count('timeout = 5') == 1, 'Unexpected config; do not guess'
    assert vsh_patch(file, 'timeout = 5', 'timeout = 15') == 1
    review.append({'path': str(file), 'before': before, 'after': vsh_read(file)})
review
```

The trusted host checks the two exact `modify` entries, the bounded before/after result
and unchanged host bytes. It then calls `commit(preview.transaction, now_ms)` and
verifies both new files. This is one preview and one exact promotion, not one
snapshot per service.

The cap+1 check is essential: glob returns no truncation flag. The original occurrence
assertion is also essential: patch's `count` is a maximum, not a required match count.
For schema-aware migrations, use an explicitly supported parser/representation rather
than pretending a text replacement validates the complete configuration language.

## Staged release generation

**Problem:** assemble generated output from templates, validate intermediate content
and publish only its final reviewed shape.

The fixture starts with `templates/service.toml` containing `channel = "dev"`.
Its shared [guest source](https://github.com/fswair/vsh/blob/main/crates/vbash/examples/staged_release.monty)
is also compiled into the Rust cookbook example:

```python
from pathlib import Path

vsh_mkdir('/workspace/release')
vsh_copy('/workspace/templates/service.toml', '/workspace/release/service.toml')
assert vsh_patch('/workspace/release/service.toml', 'channel = "dev"', 'channel = "stable"') == 1
vsh_move('/workspace/release/service.toml', '/workspace/release/app.toml')
config = Path('/workspace/release/app.toml').read_text()
assert config == 'channel = "stable"\n'
vsh_write('/workspace/release/README.txt', 'channel=stable\n')
{'config': config, 'files': len(vsh_list('/workspace/release'))}
```

Both API styles see the same overlay. The final diff contains `release`,
`release/README.txt` and `release/app.toml`; the intermediate `service.toml` is absent.
Nevertheless, the semantic rename causes **pending approval** under balanced policy.
The fixture host explicitly approves after checking the expected result and paths.

The recipe also runs a second, read-only preview and proves it cannot see the first
preview's generated directory. That probe is discarded. This distinguishes runtime
reuse from overlay reuse.

## Approval after restart

**Problem:** store a proposed change for an independent review service without
requiring the original process to stay alive.

`approve_after_restart()` uses strict policy and writes an uppercase derivative of
`input.txt`. It asserts `pending_approval`, proves commit without approval fails,
destroys the original runtime and reopens the same workspace/profile. Only after an
exact fixture review does the trusted host execute:

```python
issued = time.time_ns() // 1_000_000
runtime.approve(preview.transaction, "fixture-reviewer", issued, issued + 30_000)
committed = runtime.commit(preview.transaction, time.time_ns() // 1_000_000)
assert committed.committed
```

The principal label is not authentication. A real service must authenticate and
authorize the reviewer before calling this trusted API. Do not expose approval to the
same untrusted agent proposing the edit.

## Stale-input rejection

**Problem:** a developer edits a source file while a preview awaits application.

`reject_stale_input()` previews `input.txt` → `output.txt`, changes `input.txt` from the
trusted fixture host, and attempts exact promotion. It requires `VshStaleError`, verifies
the external edit survived and confirms `output.txt` was never created.

The next application step is a new snapshot, proposal and review. Neither the same
transaction nor a silently rerun source string is permission to overwrite the new input.

## Real MCP and CLI boundaries

```bash
uv run --no-sync python examples/native/mcp_workflow.py
uv run --no-sync python examples/native/cli_workflow.py
```

The MCP recipe uses FastMCP's client and in-process MCP transport, lists the one tool,
previews, checks the receipt and promotes the exact handle within one live server.
It is a protocol exercise, not a model benchmark or a direct helper-function call.

The CLI recipe launches separate real processes. It proves that a balanced preview
handle is lost when its process exits, then verifies explicitly requested one-shot
auto mode. See [CLI guidance](../guides/cli.md) before building a shell-based review flow.

## Adapt the recipes safely

Replace fixture roots and expected content in a trusted application. Keep scope,
policy, budgets and approval authority outside agent-controlled arguments. Retain
assertions that fail on unexpected file counts, content or decisions. Do not replace
them with an unconditional `approve()` just to make an example proceed.
