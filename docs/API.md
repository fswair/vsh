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
| `shell_preview` | Canonical shell rendering |
| `decision` | `approve`, `reject`, or `approve_with_warning` |
| `execution_eligible` | Whether the plan may reach real execution |
| `predicted_effects` | Node-level creates/updates/deletes/renames |
| `journal` | Access journal used for drift fingerprints |

## Approval

### `approve_plan(plan_id: str) -> ApprovalToken`

Bind an approval token to an immutable simulation artifact.

## Execution

### `execute_approved(approval_token: str) -> ExecutionResult`

| Field | Meaning |
|-------|---------|
| `applied` | Whether the real filesystem was mutated/verified |
| `revalidation` | Drift status (`ok` or `stale`) |
| `actual_effects` | Observed filesystem effects |
| `matches_prediction` | Whether actual effects match simulation |

Raises:

- `ValueError` — unapproved, ineligible, or already executed plan
- `KeyError` — unknown approval token

## Persistence

| Variable | Default | Purpose |
|----------|---------|---------|
| `VSH_DATA_DIR` | `~/.vsh` | JSON store root |
| `VSH_PERSIST` | `1` | Set `0` to disable disk writes |

```python
from vsh.persistence import PersistenceStore

store = PersistenceStore(root="/tmp/vsh-data")
store.save_snapshot(snapshot)
store.load_plan(plan_id)
```

## Extensions

```python
from vsh.extensions import extensions, ContentHydrator, SemanticAnalyzer

class MyHydrator:
    def hydrate(self, path: str, content_ref: str | None) -> bytes | None:
        ...

extensions.content_hydrator = MyHydrator()
```

## Agent toolset

```python
from vsh.agent import VshAgentDeps, create_vsh_function_toolset

toolset = create_vsh_function_toolset()
```

Registered tools: `vsh_search`, `vsh_get_schema`, `vsh_snapshot_workspace`,
`vsh_simulate`, `vsh_approve`, `vsh_execute_approved`.

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
