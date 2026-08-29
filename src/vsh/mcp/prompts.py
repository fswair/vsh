from __future__ import annotations as _annotations

from fastmcp import FastMCP

__all__ = ("register_codemode_prompts",)

_RUN_PROMPT = """\
Use the single `vsh_run` tool for workspace simulation and mutation.

- Send one Monty Python program containing the complete filesystem transaction.
- Use `mode="preview"` first when the user wants to inspect effects.
- To apply an auto-approved preview, call `vsh_run` again with its `transaction`, no `code`, and
  `mode="auto"`; dependency revalidation still occurs before mutation.
- Otherwise use `mode="auto"` with code only when the user requested the change; Rust policy
  still decides whether the exact virtual transaction may commit.
- Read `state`, `decision`, `changes`, `result_repr`, and `stdout` from the receipt.
- A `denied` or `pending_approval` receipt never means the host change was applied.
- Do not emulate shell commands or call an alternate simulator.
"""


def register_codemode_prompts(mcp: FastMCP) -> None:
    """Register guidance for the single native transaction tool."""

    @mcp.prompt(
        name="vsh_run_transaction",
        title="Run one VSH transaction",
        description="Compose one bounded Monty transaction over the native Rust engine.",
        tags={"native", "transaction", "vsh"},
    )
    def run_transaction() -> str:
        return _RUN_PROMPT
