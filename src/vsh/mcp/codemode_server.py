from __future__ import annotations as _annotations

import os
from pathlib import Path

from fastmcp import FastMCP

from .prompts import register_codemode_prompts
from .surface import register_vsh_surface

CODEMODE_SERVER_NAME = "vsh-codemode"

CODEMODE_INSTRUCTIONS = """\
vsh CodeMode MCP server.

Inspired by CodeMode-style agent tooling: expose a compact tool surface and let the
model discover command schemas on demand instead of loading every tool definition up
front. vsh extends that pattern with simulation, approval, and drift-aware execution.

Workflow:
  search -> get_schema -> snapshot_workspace -> simulate -> approve -> execute_approved

Rules:
- Discover commands with `search`; fetch one schema with `get_schema`.
- Snapshot once per session before simulating.
- Simulate every command before approval.
- Execute only approved, execution-eligible plans.
- Use resources for stateful context instead of bloating tool responses.

Batch simulation (`vsh_sandbox`):
- The `code` argument is a Monty Python program. Built-in helpers look like normal
  function calls: `search(query)`, `get_schema(name)`, `simulate(tool_name, params)`.
- Most programs are just those calls — one per line or in sequence, e.g.
  `simulate("vsh_list", {"path": "."})`. No extra Python required.
- When useful, you may also assign results to variables, pass them into later calls,
  slice/filter, etc.
- `simulate(...)` yields a SimulationResult dict (`plan_id`, `decision`,
  `predicted_effects`, `journal`, ...).
- Add a return expression at the end of the program; Monty treats it as the program
  result → `vsh_sandbox` field `output`. Example: `paths[:5]`. Use `print(...)` for
  debug text; that goes to `stdout`, not `output`.
- Each `simulate(...)` is recorded in `calls[]` (plan_id per step) for later
  `approve` / `execute_approved`. A compact program result does not drop plans.
- Read-command file/listing content is not in simulate output; it appears after
  `execute_approved`. Summarize from `predicted_effects` / `journal` when needed.

Example (optional chaining + program result):
  pwd = simulate("vsh_pwd", {})
  ls = simulate("vsh_list", {"path": pwd["predicted_effects"]["cwd_after"] or "."})
  paths = ls["predicted_effects"]["reads"]
  paths[:5]
"""

_CUSTOM_SECTION_HEADER = "Project-specific instructions:"

__all__ = (
    "CODEMODE_INSTRUCTIONS",
    "CODEMODE_SERVER_NAME",
    "build_codemode_instructions",
    "codemode_mcp",
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
        version="0.2.0",
    )
    register_vsh_surface(server)
    register_codemode_prompts(server)
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
