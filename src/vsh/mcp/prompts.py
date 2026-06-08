from __future__ import annotations as _annotations

from fastmcp import FastMCP

__all__ = ("register_codemode_prompts",)

_DISCOVERY_PROMPT = """\
You are using vsh in CodeMode-style discovery mode.

Do not guess command parameters. Follow this sequence:

1. Call `search` with a short query (for example "list", "mkdir", "copy").
2. Pick one `CommandSpec.name` from the results.
3. Call `get_schema` for that command only.
4. Build typed params from the returned JSON schema and examples.

Stop after schema lookup until you are ready to simulate.
"""

_SIMULATION_PROMPT = """\
Continue the vsh validation lifecycle:

1. Call `snapshot_workspace` once per workspace session and keep `snapshot_id`.
2. Call `simulate` with the registry tool name and typed params.
3. Read `decision`, `execution_eligible`, and `predicted_effects`.
4. Call `approve` only when the simulation outcome is acceptable.
5. Call `execute_approved` only when `execution_eligible` is true.

Never skip simulation for mutating commands. Never execute without approval.
"""

_WORKSPACE_READ_PROMPT = """\
For read-only workspace inspection in vsh CodeMode:

1. `search` for a read command such as `vsh_list` or `vsh_cat`.
2. `get_schema` for the chosen command.
3. `snapshot_workspace` to obtain `snapshot_id`.
4. `simulate` the read command and inspect journal/predicted effects.

Use `workspace://projection/current` when you need a cwd-oriented tree view.
"""


def register_codemode_prompts(mcp: FastMCP) -> None:
    """Register MCP prompts that teach the CodeMode discovery workflow."""

    @mcp.prompt(
        name="vsh_discover_command",
        title="Discover a vsh command",
        description="CodeMode-style search and schema lookup before simulation.",
        tags={"codemode", "discovery", "vsh"},
    )
    def discover_command() -> str:
        return _DISCOVERY_PROMPT

    @mcp.prompt(
        name="vsh_simulate_and_execute",
        title="Simulate, approve, and execute",
        description="Run the full vsh validation lifecycle after schema discovery.",
        tags={"codemode", "simulation", "approval", "vsh"},
    )
    def simulate_and_execute() -> str:
        return _SIMULATION_PROMPT

    @mcp.prompt(
        name="vsh_read_workspace",
        title="Read workspace safely",
        description="Inspect a workspace with read-only vsh commands.",
        tags={"codemode", "read", "workspace", "vsh"},
    )
    def read_workspace() -> str:
        return _WORKSPACE_READ_PROMPT
