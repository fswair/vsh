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
  -> simulate policy checks
  -> optional approval handlers (visitor)
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
| `vsh.agent` | pydantic-ai `VshCapability` + `FunctionToolset` adapter |

## Simulation

`simulate_command()` renders `shell_preview` via `command.to_shell()` (paths quoted with
`shlex.quote`), applies read or mutation logic against the snapshot graph, records an
`AccessJournal`, derives `PredictedEffects`, and runs policy checks.

Protected workspace paths (`.env`, `secrets/**`, `*.pem`, …) are rejected during
simulation via `vsh.simulate.protected_paths`. Override globs with `VSH_PROTECTED_PATTERNS`
or `VSH_PROTECTED_PATTERNS_FILE`.

Each result includes an `approval_tier` (`read_only`, `mutation`, `destructive`) and
`requires_manual_approval`. Use `auto_approve_plan()` only for read-only plans.

## Approval

Simulate policy is the **primary** approval gate. It is computed during simulation and
re-checked at approval time:

| Check | Where enforced | Typical failure |
|-------|----------------|-----------------|
| `execution_eligible` | `approve_plan()` | `ValueError` — rejected simulation, shell preview mismatch, protected paths |
| `requires_manual_approval` | `approve_plan(auto=True)` | `ValueError` — mutation/destructive tier with auto-approve |
| `approval_tier` | simulate + approve | Drives manual vs auto approval rules |

**Approval handlers** are an optional second layer. They behave like visitors:

- Registered on `extensions.approval_handlers`.
- Invoked only when the list is non-empty.
- Called after simulate policy passes, before an `ApprovalToken` is minted.
- May veto by raising `ApprovalDeniedError`.
- Do not replace simulate policy; they cannot approve ineligible plans.

When no handlers are registered, `approve_plan()` mints a token using simulate policy only.
This keeps default vsh behavior unchanged while allowing org-specific gates (ticketing,
audit, human-in-the-loop UI, external policy engines) to plug in without forking core
approval logic.

Mutation commands usually return `approve_with_warning`. Both `approve` and
`approve_with_warning` are execution-eligible unless `raw_command` fails the shell preview
match check.

`VSH_MAX_TOUCHED_PATHS` caps how many paths a single simulation may touch.

Each plan stores:

- `plan_fingerprint` — hash of command + snapshot basis
- `path_fingerprints` — touched-path fingerprints at simulation time

## Execution

`execute_approved()`:

1. Loads the approved `PlanRecord` and basis snapshot
2. Runs `revalidate_plan()` — compares live path fingerprints to the plan basis
3. On drift: refreshes snapshot nodes for diagnostics and returns `applied=False`
4. On success: dispatches the structured command through `apply_command()`
5. Read commands populate `ActualEffects.stdout` via `vsh.execute.read_output`
6. Every dispatch records `ActualEffects.execution_time_ms`
7. Updates session `cwd` when needed, records `ActualEffects`, compares to prediction

Supported real filesystem operations include navigation reads, file reads, `mkdir`, `touch`,
`mv`, `cp`, `rm`, `echo` redirection, `chmod`, `ln`, and simple in-place `sed`.

## Persistence

When `VSH_PERSIST=1` (default), snapshots and plans are written as JSON under
`$VSH_DATA_DIR` (default `~/.vsh`). Tests set `VSH_PERSIST=0` automatically.

Plan JSON stores `command_model_name` so concrete command types round-trip correctly.

## Extension hooks

Register optional collaborators on `vsh.extensions.extensions`:

- `ApprovalHandler` — optional approval visitors (`handler(ctx, item)`); see [API.md](API.md#approval-handlers)
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

Primary API: a pydantic-ai [capability](https://ai.pydantic.dev/capabilities/) bundles
instructions and tools; workspace runtime state lives on `vsh.deps`.

```python
from vsh.agent import create_vsh_agent

agent, vsh = create_vsh_agent(os.environ["MODEL_NAME"], "/path/to/workspace")
result = agent.run_sync("List files safely.", deps=vsh.deps)
```

For progressive disclosure (CodeMode-style), defer the whole workflow:

```python
from pydantic_ai import Agent
from vsh.agent import VshAgentDeps, VshCapability

vsh = VshCapability("/path/to/workspace", defer_loading=True)
agent = Agent(model, deps_type=VshAgentDeps, capabilities=[vsh])
result = agent.run_sync("List files safely.", deps=vsh.deps)
```

Legacy `toolsets=[create_vsh_function_toolset()]` remains available.

See `examples/pydantic_ai_agent_demo.py` for `.env` loading and live mode.
