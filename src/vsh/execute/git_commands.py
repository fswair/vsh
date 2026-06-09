from __future__ import annotations as _annotations

import subprocess
from pathlib import Path

from vsh.schemas import GitDiffCommand

__all__ = ("run_git_diff", "run_git_status")


def _git_root(path: str) -> Path:
    root = Path(path)
    if not (root / ".git").exists():
        msg = f"not a git repository: {path}"
        raise ValueError(msg)
    return root


def run_git_status(resolved_path: str) -> str:
    root = _git_root(resolved_path)
    result = subprocess.run(
        ["git", "-C", str(root), "status", "--porcelain"],
        check=True,
        capture_output=True,
        text=True,
    )
    return result.stdout


def run_git_diff(command: GitDiffCommand, workspace_path: str) -> str:
    root = _git_root(workspace_path)
    args = ["git", "-C", str(root), "diff"]
    if command.staged:
        args.append("--cached")
    result = subprocess.run(args, check=True, capture_output=True, text=True)
    return result.stdout
