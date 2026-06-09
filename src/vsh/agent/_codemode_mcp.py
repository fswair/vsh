from __future__ import annotations as _annotations

from typing import Any, cast

from pydantic_ai.mcp import CallToolFunc, MCPToolset
from pydantic_ai.tools import RunContext

from vsh.mcp.codemode_server import create_agent_codemode_server

from .deps import VshAgentDeps

__all__ = (
    "CODEMODE_MCP_TOOL_NAMES",
    "create_vsh_codemode_mcp_toolset",
    "inject_workspace_mcp_call",
)

CODEMODE_MCP_TOOL_NAMES = frozenset(
    {
        "apply",
        "apply_batch",
    }
)

_CODEMODE_WORKSPACE_SUPPLEMENT = """\
Workspace root is injected automatically for apply and apply_batch. Use apply_batch for
multi-step filesystem work and apply for a single command. Reuse the returned snapshot_id.
Use verbosity="full" only when debugging.
"""


def codemode_workspace_supplement() -> str:
    return _CODEMODE_WORKSPACE_SUPPLEMENT


async def inject_workspace_mcp_call(
    ctx: RunContext[VshAgentDeps],
    call_tool: CallToolFunc,
    name: str,
    args: dict[str, Any],
) -> Any:
    """Patch CodeMode MCP args with workspace state and track snapshot/plan ids."""
    patched = dict(args)
    if name in {"snapshot_workspace", "apply", "apply_batch"}:
        patched.setdefault("workspace_root", ctx.deps.workspace_root)
        patched.setdefault("cwd", ".")
    if name in {"simulate", "apply", "apply_batch"} and ctx.deps.snapshot_id is not None:
        patched.setdefault("snapshot_id", ctx.deps.snapshot_id)
    result = await call_tool(name, patched)
    if name in {"snapshot_workspace", "apply", "apply_batch"} and isinstance(result, dict):
        payload = cast(dict[str, Any], result)
        snapshot_id = payload.get("snapshot_id")
        if isinstance(snapshot_id, str):
            ctx.deps.snapshot_id = snapshot_id
    if name in {"simulate", "apply"} and isinstance(result, dict):
        payload = cast(dict[str, Any], result)
        plan_id = payload.get("plan_id")
        if isinstance(plan_id, str):
            ctx.deps.last_plan_id = plan_id
    elif name == "apply_batch" and isinstance(result, dict):
        payload = cast(dict[str, Any], result)
        steps = payload.get("steps")
        if isinstance(steps, list):
            for step in reversed(steps):
                if not isinstance(step, dict):
                    continue
                step_payload = cast(dict[str, Any], step)
                plan_id = step_payload.get("plan_id")
                if isinstance(plan_id, str):
                    ctx.deps.last_plan_id = plan_id
                    break
    return result


def create_vsh_codemode_mcp_toolset() -> MCPToolset[VshAgentDeps]:
    """Build an in-process CodeMode MCP toolset for pydantic-ai agents."""
    return MCPToolset(
        create_agent_codemode_server(),
        include_instructions=False,
        process_tool_call=inject_workspace_mcp_call,
    )
