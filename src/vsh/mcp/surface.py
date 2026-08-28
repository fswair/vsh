from __future__ import annotations as _annotations

from mcp.server import FastMCP

from . import resources, tools

__all__ = ("register_vsh_agent_surface", "register_vsh_surface")


def register_vsh_surface(mcp: FastMCP) -> None:
    """Register the compact vsh tool and resource surface on a FastMCP server."""
    mcp.add_tool(tools.search)
    mcp.add_tool(tools.get_schema)
    mcp.add_tool(tools.snapshot_workspace)
    mcp.add_tool(tools.simulate)
    mcp.add_tool(tools.approve)
    mcp.add_tool(tools.execute_approved)
    mcp.add_tool(tools.vsh_sandbox)
    mcp.add_tool(tools.apply)
    mcp.add_tool(tools.apply_batch)

    mcp.add_resource(resources.workspace_snapshot_current)
    mcp.add_resource(resources.workspace_projection_current)
    mcp.add_resource(resources.command_spec)
    mcp.add_resource(resources.simulation_record)


def register_vsh_agent_surface(mcp: FastMCP) -> None:
    """Register the minimal vsh surface for pydantic-ai agent runs."""
    mcp.add_tool(tools.apply)
    mcp.add_tool(tools.apply_batch)
