# Agent environments

VSH gives a coding agent a wide but structured workspace capability: the agent may
author one program, while the trusted host retains control over policy, budgets,
approval, revalidation, and commit.

Use `vsh-codemode` when the MCP client consumes server instructions or prompts. Use
`vsh serve` when the host supplies its own agent protocol. Both expose exactly one
normal tool.

## Recommended protocol

Give the agent this behavioral contract alongside the MCP tool:

```text
Use vsh_run for every filesystem transformation.
Start with mode="preview" and detail="full".
Never treat a returned result as proof that host files changed.
Inspect decision, changed_paths, changes, risk_flags, and deny_reason.
Promote only the exact returned transaction, through the same server process,
and only when decision is auto_approved and the changes match the stated intent.
Never bypass denied, stale, pending-approval, or recovery failures with another
filesystem tool. Ask the trusted host for review instead.
```

This is workflow guidance, not the security boundary. The Rust core still denies
protected access, enforces budgets, binds identity, and revalidates before commit.

## Agent loop

```text
task
  → write one bounded Monty program
  → vsh_run(preview, full detail)
  → inspect receipt
      denied            → revise scope or stop
      pending_approval  → request trusted review
      auto_approved     → present exact changes
  → vsh_run(transaction, auto)
  → require committed=true and retain receipt
```

## One program beats many tool calls

For related reads and writes, prefer one program:

```python
from pathlib import Path

names = []
for path in Path('/workspace/config').glob('*.txt'):
    value = path.read_text().strip()
    names.append(value)

Path('/workspace/generated/names.txt').parent.mkdir(parents=True, exist_ok=True)
Path('/workspace/generated/names.txt').write_text('\n'.join(sorted(names)) + '\n')
{'inputs': len(names), 'output': 'generated/names.txt'}
```

This keeps intermediate data in the worker, produces one canonical diff, and crosses
the MCP/PyO3 boundary once. Replacing it with dozens of read/write tool calls increases
latency, context volume, and race opportunities.

## Context-efficient receipts

Use `detail="full"` when a human or model will review individual paths. Use
`detail="compact"` for high-rate read-only work or after a workflow has another trusted
diff renderer. Compact receipts still include transaction, snapshot, diff identity,
counts, policy decision, counters, and timings.

Returned result and stdout are independently bounded. Ask the program to return only
the information needed for the next decision—counts, selected paths, or a small summary
rather than entire file contents.

## Trusted host responsibilities

The host, not the agent, should own:

- real workspace selection;
- worker executable and durable-data directory;
- policy profile and maximum allowed budget overrides;
- approval principal and expiry;
- MCP server process lifetime;
- audit receipt retention;
- operator handling for recovery conflicts.

Do not interpolate agent-controlled paths into `workspace_root`, `worker_path`, or
external state roots without an authorization layer.

## Human approval design

When policy returns pending approval, show the reviewer:

- transaction and base snapshot identities;
- stated intent;
- full canonical changes or a trusted diff renderer keyed by the diff digest;
- risk flags and changed/deleted byte/path metrics;
- program source and bounded output;
- approval expiry and principal.

The reviewer service then calls the Python or Rust SDK. Keep approval outside the same
MCP tool surface used by the model.

## Parallel agent workers

Give each independent workspace its own runtime. Native calls release the Python GIL,
and the runtime has no global execution lock. Avoid multiple agent processes sharing a
process-local preview: the handle must be promoted by the runtime instance that created
it.

For shared workspaces, expect commit-time stale failures. The correct response is a new
preview and review, not force application.

## Failure policy for agents

| Signal | Agent action |
|---|---|
| `denied` | Stop or reduce scope; do not route around VSH |
| `pending_approval` | Ask the trusted reviewer; do not auto-promote |
| `VshStaleError` / stale tool error | Re-read task context and create a new preview |
| Budget exceeded | Simplify/chunk deliberately; never request unlimited budget |
| Recovery conflict | Stop the workspace and notify an operator |
| `committed=false` | Never claim the requested host change completed |

## Good agent tasks

- update generated configuration from multiple checked-in inputs;
- apply a consistent refactor across a bounded subtree;
- normalize manifests and metadata;
- inspect workspace state and return a typed summary;
- stage a migration for human review;
- apply a reviewed exact transaction after dependency revalidation.

Tasks that require network calls, subprocesses, arbitrary package imports, or host paths
outside the workspace should be split: perform those operations in a separate trusted
tool, pass bounded data into VSH, and keep filesystem mutation inside the transaction.
