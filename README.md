# vsh

Validation-first command engine for AI agents operating on workspaces.

`vsh` is not a POSIX shell emulator. It is a **safe command planning and execution control layer** for AI agents: shell-like intent becomes typed `StructuredCommand` models, side effects are simulated on a workspace snapshot, plans are approved, drift is revalidated, and only then are mutations applied to the real filesystem. Read commands validate path access; they do not stream file contents back to the agent today.

## Install

```bash
uv sync
```

`uv sync` installs the project plus `dev` and `agent` dependency groups (including `pydantic-ai`).

For pip-only workflows:

```bash
python -m venv .venv
source .venv/bin/activate
pip install -e ".[dev,agent]"
```

## Quick start

```bash
vsh search ls
vsh schema vsh_list
vsh serve
```

## End-to-end lifecycle

```python
from vsh import LsCommand
from vsh.execute import execute_approved
from vsh.plans import approve_plan
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace

workspace = "/path/to/project"
snapshot = snapshot_workspace(workspace, cwd=workspace)
result = simulate_command(LsCommand(path=".", all=True), snapshot)
token = approve_plan(result.plan_id)
execution = execute_approved(token.token)

print(execution.applied, execution.matches_prediction)
```

Flow: **search → get_schema → snapshot → simulate → approve → execute_approved**

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — modules, lifecycle, drift detection, persistence
- [API reference](docs/API.md) — public Python/MCP surface
- [CodeMode MCP server](docs/CODEMODE.md) — discovery-first FastMCP server and prompts

## MCP

`vsh serve` starts the default FastMCP server.

For **CodeMode-style discovery** (search → schema on demand → simulate → approve → execute), use the dedicated server:

```bash
vsh serve-codemode
# or
vsh-codemode
```

See [docs/CODEMODE.md](docs/CODEMODE.md) for the CodeMode inspiration, prompts, and client config examples.

Both servers expose the same compact tool surface:

- tools: `search`, `get_schema`, `snapshot_workspace`, `simulate`, `approve`, `execute_approved`
- resources: workspace snapshot/projection, command spec cards, simulation records

## pydantic-ai agent

```bash
cp .env.example .env
# fill in MODEL_NAME and OPENROUTER_API_KEY

uv run python examples/pydantic_ai_agent_demo.py --mode live
```

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

## Configuration

| Variable | Default | Purpose |
|----------|---------|---------|
| `VSH_DATA_DIR` | `~/.vsh` | JSON persistence directory |
| `VSH_PERSIST` | `1` | Set `0` to disable disk writes |
| `MODEL_NAME` | — | Live agent model id (demo) |

## Development

```bash
ruff check src tests
ruff format src tests
ty check
basedpyright src tests
pytest tests/
```

## License

Apache-2.0
