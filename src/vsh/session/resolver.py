from __future__ import annotations as _annotations

import functools
from pathlib import Path

_STATIC_PROTECTED_ROOTS: tuple[tuple[str, str], ...] = (
    ("/", "filesystem root"),
    ("/Applications", "Applications directory"),
    ("/Library", "Library directory"),
    ("/System", "System directory"),
    ("/Users", "Users directory"),
    ("/Volumes", "Volumes directory"),
    ("/bin", "bin directory"),
    ("/dev", "device directory"),
    ("/etc", "etc directory"),
    ("/opt", "opt directory"),
    ("/private", "private directory"),
    ("/sbin", "sbin directory"),
    ("/usr", "usr directory"),
    ("/var", "var directory"),
)


@functools.lru_cache(maxsize=4096)
def _resolved_absolute_path(value: str) -> str:
    return str(Path(value).expanduser().resolve())


def _resolved_path(value: str) -> str:
    path = Path(value).expanduser()
    if path.is_absolute():
        return _resolved_absolute_path(str(path))
    return str(path.resolve())


def resolve_workspace_path(base: str, candidate: str) -> str:
    candidate_path = Path(candidate).expanduser()
    if candidate_path.is_absolute():
        return _resolved_path(str(candidate_path))
    base_resolved = _resolved_path(base)
    return str((Path(base_resolved) / candidate_path).resolve())


def is_within_workspace(path: str, workspace_root: str) -> bool:
    resolved_path = Path(_resolved_path(path)).resolve()
    resolved_root = Path(_resolved_path(workspace_root)).resolve()
    try:
        resolved_path.relative_to(resolved_root)
    except ValueError:
        return False
    return True


def is_same_path_or_ancestor(path: str, descendant: str) -> bool:
    resolved_path = _resolved_path(path)
    resolved_descendant = _resolved_path(descendant)
    try:
        Path(resolved_descendant).relative_to(resolved_path)
    except ValueError:
        return False
    return True


def get_protected_path_label(path: str) -> str | None:
    resolved_path = Path(_resolved_path(path))
    home_path = Path.home().resolve()
    dynamic_roots: tuple[tuple[Path, str], ...] = (
        (home_path, "home directory"),
        (home_path.parent, "home parent directory"),
    )
    for protected_root, label in dynamic_roots:
        if resolved_path == protected_root:
            return label
    for raw_root, label in _STATIC_PROTECTED_ROOTS:
        if resolved_path == Path(raw_root):
            return label
    return None


def ensure_safe_workspace_root(workspace_root: str) -> str:
    resolved_root = _resolved_path(workspace_root)
    protected_label = get_protected_path_label(resolved_root)
    if protected_label is not None:
        raise ValueError(f"workspace root is too broad or protected: {protected_label}")
    return resolved_root
