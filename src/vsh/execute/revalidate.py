from __future__ import annotations as _annotations

from vsh.effects import RevalidationReport
from vsh.plans.models import PlanRecord
from vsh.snapshot.fingerprint import fingerprint_path
from vsh.snapshot.models import WorkspaceSnapshot
from vsh.snapshot.refresh import refresh_snapshot_paths

__all__ = ("revalidate_plan",)


def revalidate_plan(
    record: PlanRecord,
    snapshot: WorkspaceSnapshot,
    *,
    refresh_on_drift: bool = True,
) -> tuple[RevalidationReport, WorkspaceSnapshot]:
    drift_messages: list[str] = []
    for path, expected in sorted(record.path_fingerprints.items()):
        current = fingerprint_path(path)
        if current != expected:
            drift_messages.append(
                f"path fingerprint drift at {path!r}: expected {expected!r}, found {current!r}"
            )

    if not drift_messages:
        return RevalidationReport(status="ok"), snapshot

    if not refresh_on_drift:
        return RevalidationReport(status="stale", drift_messages=drift_messages), snapshot

    refreshed_snapshot, refreshed_paths = refresh_snapshot_paths(
        snapshot,
        set(record.path_fingerprints),
    )
    return (
        RevalidationReport(
            status="stale", drift_messages=drift_messages, refreshed_paths=refreshed_paths
        ),
        refreshed_snapshot,
    )
