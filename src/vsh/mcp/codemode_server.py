from __future__ import annotations as _annotations

import os
from pathlib import Path

from fastmcp import FastMCP

from vsh import __version__

from .prompts import register_codemode_prompts
from .surface import register_vsh_agent_surface, register_vsh_surface

CODEMODE_SERVER_NAME = "vsh-codemode"

CODEMODE_INSTRUCTIONS = """\
vsh CodeMode MCP server.

The server exposes exactly one normal tool: `vsh_run`. Its `code` argument is one Monty
Python program executed against an immutable workspace snapshot and copy-on-write Rust
VirtualFs. Inside that program, use `pathlib` or the built-in `vsh_read`, `vsh_write`,
`vsh_list`, `vsh_mkdir`, `vsh_remove`, `vsh_move`, `vsh_copy`, `vsh_glob`,
`vsh_search`, and `vsh_patch` functions. These are sandbox functions, not extra MCP
tools, and both surfaces observe the same active overlay under `/workspace`.

`mode="preview"` guarantees no host mutation. `mode="auto"` asks the native policy to
commit the exact canonical diff; denied and escalated transactions remain virtual. Put
the complete multi-file operation in one program so it stays one transaction, one policy
decision, and one Python-to-Rust boundary call. To promote an auto-approved preview, pass
its returned `transaction` with no code and `mode="auto"`; VSH revalidates dependencies
before commit. Bound discovery with `max_results`. Never emulate a shell or use a second
simulation path.
"""

_CUSTOM_SECTION_HEADER = "Project-specific instructions:"

__all__ = (
    "CODEMODE_INSTRUCTIONS",
    "CODEMODE_SERVER_NAME",
    "build_codemode_instructions",
    "codemode_mcp",
    "create_agent_codemode_server",
    "create_codemode_server",
    "load_custom_instructions",
    "main",
    "run_codemode_server",
)


def build_codemode_instructions(*, custom_instructions: str | None = None) -> str:
    """Merge built-in CodeMode guidance with optional project-specific text."""
    if custom_instructions is None:
        return CODEMODE_INSTRUCTIONS

    trimmed = custom_instructions.strip()
    if not trimmed:
        return CODEMODE_INSTRUCTIONS

    return f"{CODEMODE_INSTRUCTIONS.rstrip()}\n\n---\n\n{_CUSTOM_SECTION_HEADER}\n{trimmed}\n"


def load_custom_instructions(
    *,
    inline: str | None = None,
    instructions_file: str | Path | None = None,
) -> str | None:
    """Resolve custom instructions from CLI args or environment variables."""
    parts: list[str] = []

    if instructions_file is not None:
        parts.append(Path(instructions_file).read_text(encoding="utf-8").strip())
    else:
        env_file = os.environ.get("VSH_CODEMODE_INSTRUCTIONS_FILE")
        if env_file:
            parts.append(Path(env_file).read_text(encoding="utf-8").strip())

    if inline is not None:
        parts.append(inline.strip())
    else:
        env_inline = os.environ.get("VSH_CODEMODE_INSTRUCTIONS")
        if env_inline:
            parts.append(env_inline.strip())

    merged = "\n\n".join(part for part in parts if part)
    return merged or None


def create_codemode_server(*, custom_instructions: str | None = None) -> FastMCP:
    """Build the CodeMode-oriented FastMCP server."""
    server = FastMCP(
        CODEMODE_SERVER_NAME,
        instructions=build_codemode_instructions(custom_instructions=custom_instructions),
        version=__version__,
    )
    register_vsh_surface(server)
    register_codemode_prompts(server)
    return server


def create_agent_codemode_server() -> FastMCP:
    """Build a minimal CodeMode MCP server for pydantic-ai agent runs."""
    server = FastMCP(
        CODEMODE_SERVER_NAME,
        instructions=None,
        version=__version__,
    )
    register_vsh_agent_surface(server)
    return server


def run_codemode_server(
    *,
    inline: str | None = None,
    instructions_file: str | Path | None = None,
) -> None:
    """Run the CodeMode MCP server, optionally with custom instructions."""
    custom = load_custom_instructions(inline=inline, instructions_file=instructions_file)
    if custom is None:
        codemode_mcp.run()
        return

    create_codemode_server(custom_instructions=custom).run()


codemode_mcp = create_codemode_server()


def main() -> None:
    """Run the vsh CodeMode MCP server over stdio."""
    run_codemode_server()
