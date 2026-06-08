from __future__ import annotations as _annotations

from pathlib import Path

from vsh.effects import RevalidationReport
from vsh.plans.models import PlanRecord
from vsh.snapshot.builder import snapshot_node_from_lstat
from vsh.snapshot.fingerprint import fingerprint_from_stat
from vsh.snapshot.models import WorkspaceSnapshot
from vsh.snapshot.refresh import refresh_directory_children

__all__ = ("revalidate_plan",)


def revalidate_plan(
    record: PlanRecord,
    snapshot: WorkspaceSnapshot,
    *,
    refresh_on_drift: bool = True,
) -> tuple[RevalidationReport, WorkspaceSnapshot]:
    drift_messages: list[str] = []
    refreshed_paths: list[str] = []
    nodes = dict(snapshot.nodes)
    nodes_changed = False

    for path, expected in sorted(record.path_fingerprints.items()):
        target = Path(path)
        if not target.exists():
            current = "missing"
            if current == expected:
                continue
            drift_messages.append(
                f"path fingerprint drift at {path!r}: expected {expected!r}, found {current!r}"
            )
            if refresh_on_drift:
                nodes_changed = True
                nodes.pop(path, None)
                refreshed_paths.append(path)
            continue

        stat_result = target.lstat()
        current = fingerprint_from_stat(target, stat_result)
        if current == expected:
            continue
        drift_messages.append(
            f"path fingerprint drift at {path!r}: expected {expected!r}, found {current!r}"
        )
        if not refresh_on_drift:
            continue
        nodes_changed = True
        nodes[path] = snapshot_node_from_lstat(target, stat_result)
        refreshed_paths.append(path)
        if nodes[path].kind == "dir":
            refresh_directory_children(nodes, target)

    if not drift_messages:
        return RevalidationReport(status="ok"), snapshot

    if not refresh_on_drift:
        return RevalidationReport(status="stale", drift_messages=drift_messages), snapshot

    updated_snapshot = snapshot.model_copy(update={"nodes": nodes}) if nodes_changed else snapshot
    return (
        RevalidationReport(
            status="stale", drift_messages=drift_messages, refreshed_paths=refreshed_paths
        ),
        updated_snapshot,
    )
