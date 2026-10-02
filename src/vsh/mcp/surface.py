from __future__ import annotations as _annotations

import os

from fastmcp import FastMCP

from .native_tools import _vsh_run_async

__all__ = ("register_vsh_surface",)


def register_vsh_surface(mcp: FastMCP) -> None:
    """Register the single native VSH transaction tool."""
    tool = mcp.add_tool(_vsh_run_async)
    tool.parameters["properties"]["language"]["enum"] = (
        ["monty", "bash"] if os.environ.get("VSH_ENABLE_BASH") == "1" else ["monty"]
    )
