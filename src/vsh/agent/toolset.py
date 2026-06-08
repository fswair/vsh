from __future__ import annotations as _annotations

from typing import Any

from pydantic_ai import FunctionToolset, RunContext

from vsh.mcp import tools as vsh_tools
from vsh.sandbox import SandboxPolicy

from .deps import VshAgentDeps

_VSH_TOOLSET_INSTRUCTIONS = """\
Use vsh tools to validate workspace commands before touching the real filesystem.

Workflow:
1. Call vsh_search or vsh_get_schema to discover structured commands.
2. Call vsh_snapshot_workspace once per workspace session.
3. Prefer vsh_sandbox to batch helper calls in one Monty program when that saves turns.
   Code is usually plain function(args) lines — simulate("vsh_list", {"path": "."}),
   search("touch"), etc. Extra Python (variables, chaining, slices) is optional.
   Put a return expression at the end of the program; Monty uses it as the result
   (vsh_sandbox.output). Each simulate(...) still records plan_id in calls[].
4. Or call vsh_simulate for a single command when a sandbox batch is unnecessary.
5. Only call vsh_approve and vsh_execute_approved when simulation.execution_eligible is true.

Prefer structured params from JSON schemas over raw shell strings.
"""


def create_vsh_function_toolset() -> FunctionToolset[VshAgentDeps]:
    """Build a pydantic-ai FunctionToolset wrapping the vsh MCP tool surface."""
    toolset: FunctionToolset[VshAgentDeps] = FunctionToolset(
        instructions=_VSH_TOOLSET_INSTRUCTIONS,
    )

    @toolset.tool_plain
    def vsh_search(query: str) -> list[dict[str, Any]]:
        """Find vsh command specs by name, tag, alias, or description."""
        return [spec.model_dump() for spec in vsh_tools.search(query)]

    @toolset.tool_plain
    def vsh_get_schema(name: str) -> dict[str, Any]:
        """Return the JSON schema for a structured vsh command."""
        return vsh_tools.get_schema(name)

    @toolset.tool
    def vsh_snapshot_workspace(
        ctx: RunContext[VshAgentDeps],
        cwd: str | None = None,
    ) -> dict[str, Any]:
        """Create and persist a workspace snapshot graph for simulation."""
        payload = vsh_tools.snapshot_workspace(ctx.deps.workspace_root, cwd=cwd)
        ctx.deps.snapshot_id = payload["snapshot_id"]
        return payload

    @toolset.tool
    def vsh_simulate(
        ctx: RunContext[VshAgentDeps],
        tool_name: str,
        params: dict[str, Any],
        snapshot_id: str | None = None,
    ) -> dict[str, Any]:
        """Simulate a structured command against a workspace snapshot."""
        active_snapshot_id = snapshot_id or ctx.deps.snapshot_id
        if active_snapshot_id is None:
            msg = "snapshot_id is missing; call vsh_snapshot_workspace first"
            raise ValueError(msg)
        result = vsh_tools.simulate(tool_name, active_snapshot_id, params)
        ctx.deps.last_plan_id = result["plan_id"]
        return result

    @toolset.tool
    def vsh_approve(ctx: RunContext[VshAgentDeps], plan_id: str | None = None) -> dict[str, Any]:
        """Approve a persisted simulation plan."""
        target_plan_id = plan_id or ctx.deps.last_plan_id
        if target_plan_id is None:
            msg = "plan_id is missing; call vsh_simulate first"
            raise ValueError(msg)
        token = vsh_tools.approve(target_plan_id)
        ctx.deps.last_plan_id = target_plan_id
        ctx.deps.last_approval_token = token["token"]
        return token

    @toolset.tool
    def vsh_execute_approved(
        ctx: RunContext[VshAgentDeps],
        approval_token: str | None = None,
    ) -> dict[str, Any]:
        """Execute an approved plan when simulation marked it execution-eligible."""
        token = approval_token or ctx.deps.last_approval_token
        if token is None:
            msg = "approval_token is missing; call vsh_approve first"
            raise ValueError(msg)
        return vsh_tools.execute_approved(token)

    @toolset.tool
    def vsh_sandbox(
        ctx: RunContext[VshAgentDeps],
        code: str,
        snapshot_id: str | None = None,
        policy: SandboxPolicy = "read_only",
    ) -> dict[str, Any]:
        """Run Monty code: helper calls like simulate(name, params); end expression is output."""
        active_snapshot_id = snapshot_id or ctx.deps.snapshot_id
        if active_snapshot_id is None:
            msg = "snapshot_id is missing; call vsh_snapshot_workspace first"
            raise ValueError(msg)
        return vsh_tools.vsh_sandbox(code, active_snapshot_id, policy=policy)

    return toolset
