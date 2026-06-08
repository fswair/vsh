# vsh architecture

`vsh` is a validation-first command engine for agent workspaces. Commands are typed
`StructuredCommand` models, simulated on a workspace snapshot graph, approved as immutable
plans, revalidated against filesystem drift, and only then executed on the real filesystem.

## Lifecycle

```text
search(query)
  -> CommandSpec[]

get_schema(name)
  -> JSON schema (Gemini-safe, $defs inlined)

snapshot_workspace(root, cwd?)
  -> snapshot_id + session metadata

simulate(tool_name, snapshot_id, params)
  -> SimulationResult(plan_id, shell_preview, journal, predicted_effects, decision)

approve(plan_id)
  -> ApprovalToken

execute_approved(approval_token)
  -> ExecutionResult(applied, actual_effects, revalidation, matches_prediction)
```

The canonical path is **simulate → approve → execute_approved**. There is no direct
`execute(name, params)` shortcut.

## Core modules

| Module | Responsibility |
|--------|----------------|
| `vsh.registry` | Command discovery, JSON schema export |
| `vsh.schemas` | Typed command models + shell previews |
| `vsh.snapshot` | Workspace graph builder, fingerprints, refresh |
| `vsh.simulate` | Overlay simulation, policy, predicted effects |
| `vsh.plans` | Plan store, approval tokens, fingerprints |
| `vsh.execute` | Drift revalidation + real filesystem dispatch |
| `vsh.persistence` | Optional JSON persistence under `VSH_DATA_DIR` |
| `vsh.extensions` | Optional hooks for hydration and analyzers |
| `vsh.mcp` | FastMCP tools/resources |
| `vsh.agent` | pydantic-ai `FunctionToolset` adapter |

## Simulation

`simulate_command()` renders `shell_preview` via `command.to_shell()`, applies read or
mutation logic against the snapshot graph, records an `AccessJournal`, derives
`PredictedEffects`, and runs policy checks.

Mutation commands usually return `approve_with_warning`. Both `approve` and
`approve_with_warning` are execution-eligible unless `raw_command` fails the shell preview
match check.

Each plan stores:

- `plan_fingerprint` — hash of command + snapshot basis
- `path_fingerprints` — touched-path fingerprints at simulation time

## Execution

`execute_approved()`:

1. Loads the approved `PlanRecord` and basis snapshot
2. Runs `revalidate_plan()` — compares live path fingerprints to the plan basis
3. On drift: refreshes snapshot nodes for diagnostics and returns `applied=False`
4. On success: dispatches the structured command through `apply_command()`
5. Updates session `cwd` when needed, records `ActualEffects`, compares to prediction

Supported real filesystem operations include navigation reads, file reads, `mkdir`, `touch`,
`mv`, `cp`, `rm`, `echo` redirection, `chmod`, `ln`, and simple in-place `sed`.

## Persistence

When `VSH_PERSIST=1` (default), snapshots and plans are written as JSON under
`$VSH_DATA_DIR` (default `~/.vsh`). Tests set `VSH_PERSIST=0` automatically.

Plan JSON stores `command_model_name` so concrete command types round-trip correctly.

## Extension hooks

Register optional collaborators on `vsh.extensions.extensions`:

- `ContentHydrator` — lazy file hydration for future content-aware commands
- `SemanticAnalyzer` — Python/TS checks after execution
- `ShadowWorkspaceRunner` — external verification tools

## MCP surface

Two FastMCP entrypoints share the same compact tool/resource surface via
`vsh.mcp.surface.register_vsh_surface()`:

| Command | Server | Extra |
|---------|--------|-------|
| `vsh serve` | `vsh` | tools + resources |
| `vsh serve-codemode` | `vsh-codemode` | tools + resources + CodeMode instructions + MCP prompts |

The CodeMode server is inspired by CodeMode-style discovery: agents `search` first,
then `get_schema` for a single command, instead of loading every schema into context.
See [CODEMODE.md](CODEMODE.md).

`vsh serve` / `vsh serve-codemode` expose tools:

- `search`, `get_schema`, `snapshot_workspace`, `simulate`, `approve`, `execute_approved`

Resources:

- `workspace://snapshot/current`
- `workspace://projection/current`
- `commands://spec/{name}`
- `simulations://{plan_id}`

## Agent integration

```python
from pydantic_ai import Agent
from vsh.agent import VshAgentDeps, create_vsh_function_toolset

deps = VshAgentDeps.from_path("/path/to/workspace")
agent = Agent(
    os.environ["MODEL_NAME"],
    deps_type=VshAgentDeps,
    toolsets=[create_vsh_function_toolset()],
)
```

See `examples/pydantic_ai_agent_demo.py` for `.env` loading and live mode.
