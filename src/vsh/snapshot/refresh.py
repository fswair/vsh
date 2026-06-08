from __future__ import annotations as _annotations

from pathlib import Path

from vsh.snapshot.builder import IGNORED_DIRECTORIES, node_for_path
from vsh.snapshot.models import SnapshotNode, WorkspaceSnapshot

__all__ = ("refresh_snapshot_paths",)


def refresh_snapshot_paths(
    snapshot: WorkspaceSnapshot,
    paths: set[str],
) -> tuple[WorkspaceSnapshot, list[str]]:
    refreshed: list[str] = []
    nodes = dict(snapshot.nodes)
    for raw_path in sorted(paths):
        path = Path(raw_path)
        if not path.exists():
            nodes.pop(raw_path, None)
            refreshed.append(raw_path)
            continue
        if any(part in IGNORED_DIRECTORIES for part in path.parts):
            continue
        nodes[raw_path] = node_for_path(path)
        refreshed.append(raw_path)
        if nodes[raw_path].kind == "dir":
            _refresh_directory_children(nodes, path)
    updated = snapshot.model_copy(update={"nodes": nodes})
    return updated, refreshed


def _refresh_directory_children(nodes: dict[str, SnapshotNode], directory: Path) -> None:
    root_key = str(directory.resolve())
    root_node = nodes.get(root_key)
    if root_node is None:
        return
    children: list[str] = []
    for entry in directory.iterdir():
        if entry.name in IGNORED_DIRECTORIES:
            continue
        child_key = str(entry.resolve())
        nodes[child_key] = node_for_path(entry)
        children.append(child_key)
    nodes[root_key] = root_node.model_copy(update={"children": children})
