from __future__ import annotations as _annotations

from pathlib import Path

__all__ = (
    "ALLOWED_MKDIR_PATHS",
    "ALLOWED_WRITE_PATHS",
    "NativePathError",
    "normalize_relative_path",
    "resolve_workspace_path",
    "validate_grep_scope",
    "validate_list_path",
    "validate_mkdir_path",
    "validate_read_path",
    "validate_write_path",
)

ALLOWED_MKDIR_PATHS: frozenset[str] = frozenset({"bench/output", "bench/output/"})
ALLOWED_WRITE_PATHS: frozenset[str] = frozenset(
    {
        "bench/output/summary.md",
        "bench/output/status.json",
    }
)


class NativePathError(ValueError):
    """Raised when a native filesystem tool targets a disallowed path."""


def normalize_relative_path(path: str) -> str:
    cleaned = path.strip().replace("\\", "/")
    while cleaned.startswith("./"):
        cleaned = cleaned[2:]
    return cleaned.lstrip("/")


def resolve_workspace_path(workspace_root: str, relative: str) -> Path:
    root = Path(workspace_root).resolve()
    normalized = normalize_relative_path(relative)
    if normalized.startswith("..") or "/../" in f"/{normalized}/":
        msg = f"path escapes workspace: {relative!r}"
        raise NativePathError(msg)
    resolved = (root / normalized).resolve()
    try:
        resolved.relative_to(root)
    except ValueError as exc:
        msg = f"path escapes workspace: {relative!r}"
        raise NativePathError(msg) from exc
    return resolved


def validate_mkdir_path(workspace_root: str, relative: str) -> Path:
    normalized = normalize_relative_path(relative)
    if normalized.rstrip("/") != "bench/output":
        msg = f"mkdir only allowed for bench/output, got {relative!r}"
        raise NativePathError(msg)
    return resolve_workspace_path(workspace_root, normalized)


def validate_write_path(workspace_root: str, relative: str) -> Path:
    normalized = normalize_relative_path(relative)
    if normalized not in ALLOWED_WRITE_PATHS:
        msg = f"write only allowed for {sorted(ALLOWED_WRITE_PATHS)}, got {relative!r}"
        raise NativePathError(msg)
    return resolve_workspace_path(workspace_root, normalized)


def validate_read_path(workspace_root: str, relative: str) -> Path:
    resolved = resolve_workspace_path(workspace_root, relative)
    if "bench/output" not in str(resolved.relative_to(Path(workspace_root).resolve())).replace(
        "\\", "/"
    ):
        msg = f"read only allowed under bench/output, got {relative!r}"
        raise NativePathError(msg)
    return resolved


def validate_list_path(workspace_root: str, relative: str) -> Path:
    normalized = normalize_relative_path(relative).rstrip("/")
    if normalized not in {"bench/output", "bench"}:
        msg = f"list_dir only allowed for bench/output (or bench), got {relative!r}"
        raise NativePathError(msg)
    return resolve_workspace_path(workspace_root, normalized)


def validate_grep_scope(workspace_root: str, relative: str) -> Path:
    normalized = normalize_relative_path(relative)
    if normalized in {"", "."}:
        return Path(workspace_root).resolve()
    if normalized not in {"bench", "bench/output", "bench/output/"}:
        msg = f"grep scope must be . or bench/output, got {relative!r}"
        raise NativePathError(msg)
    return resolve_workspace_path(workspace_root, normalized)
