from __future__ import annotations as _annotations

from .models import WorkspaceSnapshot


def project_snapshot(snapshot: WorkspaceSnapshot, cwd: str | None = None) -> dict[str, object]:
    current_cwd = cwd or snapshot.session.cwd_logical
    visible_nodes = {
        path: node.model_dump()
        for path, node in snapshot.nodes.items()
        if path.startswith(snapshot.session.workspace_root)
    }
    return {
        "snapshot_id": snapshot.snapshot_id,
        "cwd": current_cwd,
        "workspace_root": snapshot.session.workspace_root,
        "nodes": visible_nodes,
    }
