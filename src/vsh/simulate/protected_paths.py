from __future__ import annotations as _annotations

import fnmatch
import os
from pathlib import Path, PurePosixPath

__all__ = (
    "DEFAULT_PROTECTED_PATTERNS",
    "get_protected_workspace_path_reason",
    "load_protected_patterns",
    "matches_protected_pattern",
    "workspace_relative_path",
)

DEFAULT_PROTECTED_PATTERNS: tuple[str, ...] = (
    ".env",
    ".env.*",
    "**/.env",
    "**/.env.*",
    "**/secrets/**",
    "**/id_rsa",
    "**/id_rsa.pub",
    "*.pem",
    "*.key",
    "**/*.pem",
    "**/*.key",
    "**/credentials.json",
    "**/*_credentials.json",
    "**/.ssh/**",
)


def load_protected_patterns() -> tuple[str, ...]:
    """Load protected path globs from env or fall back to defaults."""
    env_patterns = os.environ.get("VSH_PROTECTED_PATTERNS")
    if env_patterns:
        patterns = tuple(part.strip() for part in env_patterns.split(",") if part.strip())
        if patterns:
            return patterns

    env_file = os.environ.get("VSH_PROTECTED_PATTERNS_FILE")
    if env_file:
        lines = Path(env_file).read_text(encoding="utf-8").splitlines()
        patterns = tuple(
            line.strip() for line in lines if line.strip() and not line.startswith("#")
        )
        if patterns:
            return patterns

    return DEFAULT_PROTECTED_PATTERNS


def workspace_relative_path(path: str, workspace_root: str) -> str | None:
    """Return a posix-style workspace-relative path when inside the root."""
    resolved_path = Path(path).expanduser().resolve()
    resolved_root = Path(workspace_root).expanduser().resolve()
    try:
        relative = resolved_path.relative_to(resolved_root)
    except ValueError:
        return None
    return relative.as_posix()


def matches_protected_pattern(relative_path: str, patterns: tuple[str, ...]) -> bool:
    """Match a workspace-relative path against configured protected globs."""
    candidate = PurePosixPath(relative_path)
    basename = candidate.name
    for pattern in patterns:
        if "/" not in pattern and "**" not in pattern:
            if fnmatch.fnmatchcase(basename, pattern):
                return True
            continue
        if candidate.match(pattern):
            return True
        if _match_globstar_pattern(relative_path, pattern):
            return True
    return False


def _match_globstar_pattern(relative_path: str, pattern: str) -> bool:
    if "**" not in pattern:
        return False
    normalized = pattern.strip("/")
    parts = PurePosixPath(relative_path).parts
    if normalized.endswith("/**"):
        anchor = normalized.removesuffix("/**").removeprefix("**/")
        if anchor and anchor in parts:
            return True
    if normalized.startswith("**/"):
        suffix = normalized.removeprefix("**/")
        if relative_path == suffix or relative_path.endswith(f"/{suffix}"):
            return True
    return fnmatch.fnmatchcase(relative_path, pattern.replace("**", "*"))


def get_protected_workspace_path_reason(
    path: str,
    workspace_root: str,
    *,
    patterns: tuple[str, ...] | None = None,
) -> str | None:
    """Return a rejection reason when a path matches protected workspace globs."""
    relative = workspace_relative_path(path, workspace_root)
    if relative is None:
        return None
    active_patterns = load_protected_patterns() if patterns is None else patterns
    if matches_protected_pattern(relative, active_patterns):
        return f"path matches protected workspace pattern: {relative}"
    return None
