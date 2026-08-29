# MCP server

VSH's MCP adapter exposes exactly one normal tool: `vsh_run`. One multi-file Monty
program becomes one Rust transaction, one policy decision, and one Python/Rust boundary
crossing.

## Install and start

```bash
uv add 'vbash[mcp]==0.3.0'
uv run vsh serve
```

The transport is stdio. The server process working directory is the default workspace
for calls that omit `workspace_root`.

For coding agents, `vsh-codemode` exposes the same one-tool surface and additionally
publishes workflow instructions plus the `vsh_run_transaction` MCP prompt:

```bash
uv run vsh-codemode
```

From a source checkout:

```bash
uv sync --frozen --all-groups --extra mcp
uv run vsh serve
```

## Generic MCP client configuration

Clients that accept the common `mcpServers` launch shape can run the project-local
binary:

```json
{
  "mcpServers": {
    "vsh": {
      "command": "uv",
      "args": ["run", "vsh", "serve"],
      "cwd": "/absolute/path/to/project"
    }
  }
}
```

Replace the final two arguments with `vsh-codemode` when the client consumes MCP server
instructions/prompts:

```json
{
  "mcpServers": {
    "vsh": {
      "command": "uv",
      "args": ["run", "vsh-codemode"],
      "cwd": "/absolute/path/to/project",
      "env": {
        "VSH_CODEMODE_INSTRUCTIONS_FILE": ".vsh/agent-rules.md"
      }
    }
  }
}
```

For an installed wheel, point `command` at the environment's `vsh` executable and keep
the workspace as `cwd`. Never let an untrusted agent choose the worker executable or
durable state directory.

## `vsh_run`

```text
vsh_run(
    code: str | None = None,
    *,
    transaction: str | None = None,
    workspace_root: str | None = None,
    intent: str | None = None,
    mode: Literal["preview", "auto"] = "preview",
    policy: Literal["balanced", "strict", "paranoid"] = "balanced",
    detail: Literal["compact", "full"] = "compact",
    budget: BudgetOverrides | None = None,
) -> dict[str, object]
```

### Inputs

| Field | Rule |
|---|---|
| `code` | Required for a new transaction; exact Monty source |
| `transaction` | Used instead of code to promote one preview |
| `workspace_root` | Existing directory; defaults to server `cwd` |
| `intent` | Trusted context bound into transaction identity |
| `mode` | Preview or deterministic auto mode |
| `policy` | Built-in deterministic profile |
| `detail` | Compact receipt or full canonical path list |
| `budget` | Optional overrides for the 13 native execution ceilings |

Passing both `code` and `transaction` is rejected. A transaction handle may only be
resumed with `mode="auto"`.

### Output envelope

| Group | Fields |
|---|---|
| Identity | `transaction`, `base_snapshot`, `diff` |
| Decision | `state`, `decision`, `risk_flags`, `deny_reason` |
| Effects | `changed_paths`, `changes` |
| Guest output | `result_repr`, `result_truncated`, `stdout`, `stdout_truncated` |
| Counters | `os_calls`, `read_bytes`, `write_bytes`, `directory_entries`, `output_bytes`, `denied_accesses`, `result_bytes` |
| Commit | `committed`, `operations`, `verified_paths`, `cleanup_pending` |
| Profiling | `timings_ns` |

The MCP adapter deliberately projects the native typed result as bounded `result_repr`
rather than an arbitrary JSON value. `result_repr` and `stdout` are capped to a 64 KiB
inline representation even when the native request budget is larger. The response marks
truncation rather than silently growing agent context. Python SDK callers still receive
the native typed value through `Receipt.result`.

## Preview, inspect, promote

First tool call:

```json
{
  "code": "from pathlib import Path\nPath('/workspace/generated.txt').write_text('ready\\n')\n{'files': 1}",
  "intent": "Generate reviewed status file",
  "mode": "preview",
  "policy": "balanced",
  "detail": "full",
  "budget": {
    "max_duration_ms": 300,
    "max_os_calls": 1000,
    "max_write_bytes": 1048576
  }
}
```

If the result is `auto_approved` and the exact change list is acceptable, make a second
call through the same live server process:

```json
{
  "transaction": "<transaction from preview>",
  "mode": "auto"
}
```

VSH persists the exact process-local artifact, consumes its reservation, revalidates
dependencies, commits, and verifies. It does not rerun the Monty program.

## Pending approval over MCP

The compact MCP surface intentionally does not expose an approval-minting tool. A
`pending_approval` receipt remains non-mutating. If an organization needs independent
approval, a trusted host should call the Python or Rust SDK `approve` method after its
own reviewer/identity check, or expose a separately governed service boundary.

This prevents a tool-using model from both requesting and granting its own approval.

## Agent-specific instructions

The CodeMode entry point accepts trusted project guidance through:

| Variable | Purpose |
|---|---|
| `VSH_CODEMODE_INSTRUCTIONS_FILE` | UTF-8 project rules file |
| `VSH_CODEMODE_INSTRUCTIONS` | Short inline rules appended after file content |

The built-in instructions always remain first and describe preview, exact promotion,
non-mutating denial/escalation, and the one-program/one-transaction rule. Custom text
does not alter Rust policy or grant filesystem authority.

## Runtime reuse and process lifetime

The adapter caches runtimes by resolved workspace, policy, and worker identity. This
reuses supervised clean workers and preserves bounded auto-approved preview handles.

- Restarting the MCP process loses process-local auto-approved previews.
- Pending-approval artifacts are durable and survive restart.
- Changing workspace, policy, or `VSH_MONTY_WORKER` selects another runtime identity.
- The cache is bounded; it is not an unbounded workspace registry.

## Operational errors

MCP clients should treat tool errors as terminal for that transaction. In particular:

- budget/execution errors require a new reviewed request;
- stale commit requires a new preview against current workspace state;
- state/replay errors must not be retried with the same transaction;
- recovery/internal errors require host/operator attention.

Do not fall back to direct filesystem tools after a VSH denial. That would bypass the
policy and evidence boundary the agent was configured to use.
