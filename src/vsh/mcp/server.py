from __future__ import annotations as _annotations

from fastmcp import FastMCP

from .surface import register_vsh_surface

mcp = FastMCP("vsh")
register_vsh_surface(mcp)
