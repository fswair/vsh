from __future__ import annotations as _annotations

from pathlib import Path

from vsh.plans.models import SimulationResult
from vsh.snapshot.models import SnapshotNode, WorkspaceSnapshot

__all__ = ("advance_snapshot",)


def advance_snapshot(snapshot: WorkspaceSnapshot, result: SimulationResult) -> WorkspaceSnapshot:
    """Apply a successful simulation onto an in-memory snapshot for chained sandbox calls."""
    nodes = {path: node.model_copy() for path, node in snapshot.nodes.items()}
    predicted = result.predicted_effects

    for path in predicted.deletes:
        _remove_node(nodes, path)

    for src, dst in predicted.renames:
        _rename_node(nodes, src, dst)

    for path in predicted.creates:
        _ensure_node(nodes, path, kind=_infer_kind(path, default="file"))

    for path in predicted.updates:
        if path in nodes:
            node = nodes[path]
            nodes[path] = node.model_copy(update={"revision": node.revision + 1})
        else:
            _ensure_node(nodes, path, kind=_infer_kind(path, default="file"))

    session = snapshot.session
    if predicted.cwd_after is not None:
        session = session.with_cwd(predicted.cwd_after)

    return snapshot.model_copy(update={"nodes": nodes, "session": session})


def _infer_kind(path: str, *, default: str) -> str:
    suffix = Path(path).suffix
    if suffix == "" and default == "file":
        return "file"
    return default


def _ensure_node(nodes: dict[str, SnapshotNode], path: str, *, kind: str) -> None:
    if path in nodes:
        return
    parent = str(Path(path).parent)
    if parent not in nodes and parent != path:
        _ensure_node(nodes, parent, kind="dir")
    nodes[path] = SnapshotNode(path=path, parent=parent if parent != path else None, kind=kind)
    if parent in nodes and parent != path:
        parent_node = nodes[parent]
        children = list(parent_node.children)
        if path not in children:
            children.append(path)
            nodes[parent] = parent_node.model_copy(update={"children": children})


def _remove_node(nodes: dict[str, SnapshotNode], path: str) -> None:
    node = nodes.pop(path, None)
    if node is None or node.parent is None:
        return
    parent = nodes.get(node.parent)
    if parent is None:
        return
    children = [child for child in parent.children if child != path]
    nodes[node.parent] = parent.model_copy(update={"children": children})


def _rename_node(nodes: dict[str, SnapshotNode], src: str, dst: str) -> None:
    node = nodes.pop(src, None)
    if node is None:
        _ensure_node(nodes, dst, kind=_infer_kind(dst, default="file"))
        return
    nodes[dst] = node.model_copy(update={"path": dst, "parent": str(Path(dst).parent)})
    if node.parent and node.parent in nodes:
        parent = nodes[node.parent]
        children = [dst if child == src else child for child in parent.children]
        nodes[node.parent] = parent.model_copy(update={"children": children})
