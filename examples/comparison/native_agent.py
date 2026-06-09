from __future__ import annotations as _annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

from pydantic_ai import Agent, RunContext

from .native_path_guard import (
    NativePathError,
    validate_grep_scope,
    validate_list_path,
    validate_mkdir_path,
    validate_read_path,
    validate_write_path,
)

__all__ = ("NativeAgentDeps", "NATIVE_TOOL_NAMES", "create_native_fs_agent")

NATIVE_TOOL_NAMES: frozenset[str] = frozenset(
    {
        "mkdir",
        "write_file",
        "read_file",
        "grep",
        "list_dir",
    }
)


@dataclass(kw_only=True)
class NativeAgentDeps:
    workspace_root: str


def _error(message: str) -> str:
    return f"error: {message}"


def create_native_fs_agent(
    model: Any,
    workspace_root: str | Path,
    *,
    instructions: str,
    **agent_kwargs: Any,
) -> Agent[NativeAgentDeps, str]:
    """Build a native agent with structured filesystem tools (not a shell REPL)."""
    agent: Agent[NativeAgentDeps, str] = Agent(
        model,
        deps_type=NativeAgentDeps,
        instructions=instructions,
        **agent_kwargs,
    )

    @agent.tool
    def mkdir(ctx: RunContext[NativeAgentDeps], path: str) -> str:
        """Create bench/output directory (parents=True). Only bench/output is allowed."""
        try:
            target = validate_mkdir_path(ctx.deps.workspace_root, path)
        except NativePathError as exc:
            return _error(str(exc))
        target.mkdir(parents=True, exist_ok=True)
        return f"created: {target.relative_to(Path(ctx.deps.workspace_root).resolve())}"

    @agent.tool
    def write_file(ctx: RunContext[NativeAgentDeps], path: str, content: str) -> str:
        """Write UTF-8 text to bench/output/summary.md or bench/output/status.json."""
        try:
            target = validate_write_path(ctx.deps.workspace_root, path)
        except NativePathError as exc:
            return _error(str(exc))
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content, encoding="utf-8")
        return f"wrote {target.stat().st_size} bytes to {path}"

    @agent.tool
    def read_file(ctx: RunContext[NativeAgentDeps], path: str) -> str:
        """Read a UTF-8 file under bench/output."""
        try:
            target = validate_read_path(ctx.deps.workspace_root, path)
        except NativePathError as exc:
            return _error(str(exc))
        if not target.is_file():
            return _error(f"file not found: {path}")
        return target.read_text(encoding="utf-8")

    @agent.tool
    def grep(
        ctx: RunContext[NativeAgentDeps],
        pattern: str,
        path: str = ".",
        *,
        recursive: bool = True,
    ) -> list[dict[str, str | int]]:
        """Search for pattern under workspace scope (. or bench/output)."""
        if not pattern.strip():
            return [{"error": "pattern must not be empty"}]  # type: ignore[list-item]
        try:
            root = validate_grep_scope(ctx.deps.workspace_root, path)
        except NativePathError as exc:
            return [{"error": str(exc)}]  # type: ignore[list-item]

        hits: list[dict[str, str | int]] = []
        workspace = Path(ctx.deps.workspace_root).resolve()
        if root.is_file():
            file_candidates = [root]
        elif recursive:
            file_candidates = sorted(path for path in root.rglob("*") if path.is_file())
        else:
            file_candidates = sorted(path for path in root.iterdir() if path.is_file())
        for candidate in file_candidates:
            try:
                lines = candidate.read_text(encoding="utf-8").splitlines()
            except (OSError, UnicodeDecodeError):
                continue
            rel = str(candidate.relative_to(workspace))
            for line_no, line in enumerate(lines, start=1):
                if pattern in line:
                    hits.append({"path": rel, "line": line_no, "text": line})
        return hits

    @agent.tool
    def list_dir(ctx: RunContext[NativeAgentDeps], path: str) -> list[str]:
        """List entries in bench/output (or bench)."""
        try:
            target = validate_list_path(ctx.deps.workspace_root, path)
        except NativePathError as exc:
            return [_error(str(exc))]
        if not target.is_dir():
            return [_error(f"directory not found: {path}")]
        return sorted(entry.name for entry in target.iterdir())

    return agent


# Backward-compatible alias for imports that still use the old name.
create_native_bash_agent = create_native_fs_agent
