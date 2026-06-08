from __future__ import annotations as _annotations

import os
import stat as stat_module
import time
import uuid
from pathlib import Path

from vsh.runtime import runtime
from vsh.session import SessionState

from .models import SnapshotNode, WorkspaceSnapshot

IGNORED_DIRECTORIES = frozenset(
    {
        ".git",
        ".pytest_cache",
        ".ruff_cache",
        "__pycache__",
        ".mypy_cache",
        ".venv",
        "node_modules",
        "dist",
        "build",
        "target",
    }
)


def snapshot_workspace(workspace_root: str, cwd: str | None = None) -> WorkspaceSnapshot:
    session = SessionState.from_workspace_root(workspace_root, cwd=cwd)
    root = Path(session.workspace_root)
    nodes = _build_nodes(root)
    snapshot = WorkspaceSnapshot(
        snapshot_id=f"snap_{uuid.uuid4().hex[:12]}",
        session=session,
        generated_at_ns=time.time_ns(),
        nodes=nodes,
    )
    runtime.record_snapshot(snapshot)
    return snapshot


def _build_nodes(root: Path) -> dict[str, SnapshotNode]:
    nodes: dict[str, SnapshotNode] = {}
    for current_root, dirnames, filenames in os.walk(root):
        current_path = Path(current_root)
        dirnames[:] = [name for name in dirnames if name not in IGNORED_DIRECTORIES]
        node = node_for_path(current_path)
        nodes[str(current_path)] = node
        for child_dir in dirnames:
            node.children.append(str(current_path / child_dir))
        for filename in filenames:
            child_path = current_path / filename
            nodes[str(child_path)] = node_for_path(child_path)
            node.children.append(str(child_path))
    if str(root) not in nodes:
        nodes[str(root)] = node_for_path(root)
    return nodes


def node_for_path(path: Path) -> SnapshotNode:
    return snapshot_node_from_lstat(path, path.lstat())


def snapshot_node_from_lstat(path: Path, stat_result: os.stat_result) -> SnapshotNode:
    mode = stat_result.st_mode
    if stat_module.S_ISLNK(mode):
        kind = "symlink"
    elif stat_module.S_ISDIR(mode):
        kind = "dir"
    else:
        kind = "file"
    parent = str(path.parent) if path.parent != path else None
    content_ref = None
    size = None
    if stat_module.S_ISREG(mode):
        size = stat_result.st_size
        content_ref = f"opaque:{path}:{stat_result.st_size}:{stat_result.st_mtime_ns}"
    return SnapshotNode(
        path=str(path),
        parent=parent,
        kind=kind,
        size=size,
        mode=stat_result.st_mode,
        mtime_ns=stat_result.st_mtime_ns,
        content_ref=content_ref,
    )
