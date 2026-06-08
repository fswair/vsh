# vsh API reference

## Python entry points

```python
from vsh import search, get_schema, registry, LsCommand
from vsh.snapshot.builder import snapshot_workspace
from vsh.simulate.engine import simulate_command
from vsh.plans import approve_plan
from vsh.execute import execute_approved
```

## Discovery

### `search(query: str) -> list[CommandSpec]`

Find commands by name, tag, alias, or description. Empty query returns all commands sorted.

### `get_schema(name: str) -> dict[str, Any]`

Return the JSON schema for a command. Local `$defs` / `$ref` entries are inlined for
Gemini/OpenRouter compatibility.

## Snapshot

### `snapshot_workspace(workspace_root: str, cwd: str | None = None) -> WorkspaceSnapshot`

Build an in-memory workspace graph. Ignored directories include `.git`, `.venv`,
`node_modules`, `dist`, `build`, and `target`.

### `project_snapshot(snapshot) -> dict`

Return a cwd-oriented projection for agents.

## Simulation

### `simulate_command(command, snapshot) -> SimulationResult`

Fields of interest:

| Field | Meaning |
|-------|---------|
| `plan_id` | Stable plan identifier |
| `shell_preview` | Canonical shell rendering (`shlex.quote` for paths) |
| `decision` | `approve`, `reject`, or `approve_with_warning` |
| `execution_eligible` | Whether the plan may reach real execution |
| `approval_tier` | `read_only`, `mutation`, or `destructive` |
| `requires_manual_approval` | Whether `auto_approve_plan` is allowed |
| `predicted_effects` | Node-level creates/updates/deletes/renames |
| `journal` | Access journal used for drift fingerprints |
| `simulation_time_ms` | Wall-clock time spent in `simulate_command` |

## Approval

Approval has two layers:

1. **Built-in simulate policy** — decided during `simulate_command()` and enforced again in
   `approve_plan()` (`execution_eligible`, `requires_manual_approval`, `approval_tier`).
   This is the primary approval gate. Every approval path (Python, MCP, agent tools) goes
   through it.
2. **Optional approval handlers** — visitor callbacks registered on
   `extensions.approval_handlers`. They run only when registered. When the list is empty,
   approval proceeds under simulate policy alone. Handlers may veto approval; they never
   replace or bypass simulate policy.

```text
approve_plan(plan_id)
  -> load PlanRecord
  -> enforce simulate policy (ValueError on failure)
  -> run approval handlers, if any (ApprovalDeniedError on veto)
  -> mint ApprovalToken
```

### `approve_plan(plan_id: str, *, auto: bool = False) -> ApprovalToken`

Bind an approval token to an immutable simulation artifact. Set `auto=True` only for
read-only plans where `requires_manual_approval` is `False`.

Raises:

- `ValueError` — plan is not execution-eligible, or `auto=True` on a plan that requires
  manual approval
- `ApprovalDeniedError` — a registered approval handler vetoed the request

### `auto_approve_plan(plan_id: str) -> ApprovalToken`

Convenience wrapper for `approve_plan(plan_id, auto=True)`. Raises `ValueError` for
mutation or destructive tiers.

### Approval handlers

Register optional visitors before starting MCP or agent surfaces:

```python
from vsh.extensions import ApprovalHandler, extensions
from vsh.plans import ApprovalContext, ApprovalDeniedError, ApproveItem

def require_ticket(ctx: ApprovalContext, item: ApproveItem) -> None:
    if item.requires_manual_approval and ctx.auto:
        raise ApprovalDeniedError(
            "auto-approve blocked by org policy",
            plan_id=item.plan_id,
        )

extensions.approval_handlers.append(require_ticket)
```

Handler signature:

```python
def handler(ctx: ApprovalContext, item: ApproveItem) -> None: ...
```

| Type | Fields / meaning |
|------|------------------|
| `ApprovalContext` | `auto: bool` — `True` when approval came from `auto_approve_plan()` |
| `ApproveItem` | `plan_id`, `snapshot_id`, `workspace_root`, `shell_preview`, `decision`, `approval_tier`, `requires_manual_approval`, `execution_eligible`, `execution_eligibility_reason`, `plan_fingerprint` |

Behavior:

- Handlers are invoked in registration order after simulate policy checks pass and before
  `plan_store.approve()` mints a token.
- A handler returns normally to allow approval to continue.
- Raise `ApprovalDeniedError` to block token minting. The optional `plan_id=` keyword is
  stored on the exception for logging.
- Handlers cannot grant approval for ineligible plans; `approve_plan()` rejects those first.
- With no handlers registered, `approve_plan()` behavior is unchanged from built-in policy
  alone.

All entry points share the same hook: `approve_plan()`, `auto_approve_plan()`, MCP
`approve`, and agent `vsh_approve`.

## Execution

### `execute_approved(approval_token: str) -> ExecutionResult`

| Field | Meaning |
|-------|---------|
| `applied` | Whether the real filesystem was mutated/verified |
| `revalidation` | Drift status (`ok` or `stale`) |
| `actual_effects` | Observed filesystem effects |
| `actual_effects.stdout` | Captured output for read commands (`cat`, `grep`, `ls`, …) |
| `matches_prediction` | Whether actual effects match simulation |
| `total_time_ms` | Full `execute_approved` wall time |
| `revalidation_time_ms` | Drift revalidation segment |
| `apply_time_ms` | Real filesystem dispatch segment |

Raises:

- `ValueError` — unapproved, ineligible, or already executed plan
- `KeyError` — unknown approval token

## Persistence

| Variable | Default | Purpose |
|----------|---------|---------|
| `VSH_DATA_DIR` | `~/.vsh` | JSON store root |
| `VSH_PERSIST` | `1` | Set `0` to disable disk writes |
| `VSH_PROTECTED_PATTERNS` | built-in defaults | Comma-separated protected path globs |
| `VSH_PROTECTED_PATTERNS_FILE` | — | Newline-separated protected globs file |
| `VSH_MAX_TOUCHED_PATHS` | `500` | Simulation limit for touched paths |

```python
from vsh.persistence import PersistenceStore

store = PersistenceStore(root="/tmp/vsh-data")
store.save_snapshot(snapshot)
store.load_plan(plan_id)
```

## Extensions

```python
from vsh.extensions import extensions, ApprovalHandler, ContentHydrator, SemanticAnalyzer
from vsh.plans import ApprovalContext, ApprovalDeniedError, ApproveItem

class MyHydrator:
    def hydrate(self, path: str, content_ref: str | None) -> bytes | None:
        ...

extensions.content_hydrator = MyHydrator()
```

See [Approval handlers](#approval-handlers) for the optional approval visitor API.

## Agent toolset

```python
from vsh.agent import VshAgentDeps, create_vsh_function_toolset

toolset = create_vsh_function_toolset()
```

Registered tools: `vsh_search`, `vsh_get_schema`, `vsh_snapshot_workspace`,
`vsh_simulate`, `vsh_sandbox`, `vsh_approve`, `vsh_execute_approved`.

## Sandbox (Monty batch execution)

```python
from vsh.sandbox import SandboxPolicy, run_vsh_sandbox

result = run_vsh_sandbox(
    code='simulate("vsh_touch", {"path": "x.txt"})\nreturn "ok"',
    snapshot_id=snapshot.snapshot_id,
    policy="read_write",
)
```

`SandboxResult` fields:

| Field | Meaning |
|-------|---------|
| `output` | Monty return value |
| `stdout` | Captured `print()` output |
| `calls` | Ordered `SandboxCallRecord` list (tool, params, plan_id, decision) |
| `policy` | Policy applied for the run |
| `error` | Syntax/runtime/policy error message when the run failed |
| `execution_time_ms` | Wall-clock sandbox duration |

MCP tool: `vsh_sandbox(code, snapshot_id, policy="read_only")`.

## MCP servers

### Default server

```bash
vsh serve
```

Server name: `vsh`

### CodeMode server

```bash
vsh serve-codemode
# equivalent:
vsh-codemode
```

Server name: `vsh-codemode`

Includes CodeMode workflow `instructions` and MCP prompts (`vsh_discover_command`,
`vsh_simulate_and_execute`, `vsh_read_workspace`). Append project-specific guidance via
`-i` / `-f`, `VSH_CODEMODE_INSTRUCTIONS`, or `create_codemode_server(custom_instructions=...)`.
See [CODEMODE.md](CODEMODE.md).

Both servers register the same tools and resources.

## CLI

```bash
vsh search ls
vsh schema vsh_list
vsh names grep
vsh serve
vsh serve-codemode
```
