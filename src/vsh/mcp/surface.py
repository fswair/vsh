from __future__ import annotations as _annotations

import os
from pathlib import Path
from typing import Literal

from fastmcp import FastMCP

from vsh._response import DEFAULT_RESPONSE_BYTES, MIN_RESPONSE_BYTES, bound_mcp_response

from .native_tools import BudgetOverrides, DetailName, PolicyName, RunModeName, _vsh_run_async

__all__ = ("register_vsh_surface",)


def register_vsh_surface(
    mcp: FastMCP,
    *,
    workspace_root: str | None = None,
    policy: PolicyName = "balanced",
    budget: BudgetOverrides | None = None,
    max_response_bytes: int = DEFAULT_RESPONSE_BYTES,
    retain_read_only_previews: bool = False,
) -> None:
    """Register one tool with authority fixed by the trusted server host.

    Capture the workspace at registration: a later cwd change or model argument
    cannot select another root. Python callers may configure policy and budgets;
    neither is part of the model-facing schema.
    """
    workspace = Path(workspace_root or os.environ.get("VSH_WORKSPACE_ROOT") or os.getcwd()).resolve(
        strict=True
    )
    if not workspace.is_dir():
        raise NotADirectoryError(f"workspace root is not a directory: {workspace}")
    if policy not in {"balanced", "strict", "paranoid"}:
        raise ValueError(f"unknown policy profile: {policy!r}")
    host_budget = None if budget is None else budget.copy()
    if max_response_bytes < MIN_RESPONSE_BYTES:
        raise ValueError(f"max_response_bytes must be at least {MIN_RESPONSE_BYTES}")

    async def vsh_run(
        code: str | None = None,
        *,
        language: Literal["monty", "bash"] = "monty",
        transaction: str | None = None,
        intent: str | None = None,
        mode: RunModeName = "preview",
        detail: DetailName = "compact",
    ) -> dict[str, object]:
        """Run a transaction in the host-authorized workspace; preview never mutates host files."""
        payload = await _vsh_run_async(
            code,
            language=language,
            transaction=transaction,
            intent=intent,
            mode=mode,
            detail=detail,
            workspace_root=str(workspace),
            policy=policy,
            budget=host_budget,
            discard_read_only_preview=not retain_read_only_previews,
        )
        # Bash streams are byte-authoritative; avoid repeating them as repr,
        # lossy text, and base64 in the same model response.
        if payload["bash"] is not None:
            payload.update(result_repr="", stdout="", stderr="")
        return bound_mcp_response(payload, max_response_bytes)

    tool = mcp.add_tool(vsh_run)
    tool.parameters["properties"]["language"]["enum"] = (
        ["monty", "bash"] if os.environ.get("VSH_ENABLE_BASH") == "1" else ["monty"]
    )
