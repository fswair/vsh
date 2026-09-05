from __future__ import annotations as _annotations

from fastmcp import FastMCP

from .native_tools import vsh_run

__all__ = ("register_vsh_surface",)


def register_vsh_surface(mcp: FastMCP) -> None:
    """Register the single native VSH transaction tool."""
    mcp.add_tool(vsh_run)
