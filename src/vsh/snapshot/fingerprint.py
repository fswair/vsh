from __future__ import annotations as _annotations

import os
import stat as stat_module
from pathlib import Path

from vsh.simulate.models import AccessJournal, PredictedEffects
from vsh.snapshot.models import SnapshotNode, WorkspaceSnapshot

__all__ = (
    "collect_touched_paths",
    "fingerprint_from_stat",
    "fingerprint_node",
    "fingerprint_path",
    "fingerprints_for_paths",
)


def fingerprint_node(node: SnapshotNode) -> str:
    return f"{node.kind}:{node.size}:{node.mtime_ns}:{node.mode}:{node.revision}"


def fingerprint_from_stat(path: Path, stat_result: os.stat_result) -> str:
    mode = stat_result.st_mode
    if stat_module.S_ISLNK(mode):
        kind = "symlink"
    elif stat_module.S_ISDIR(mode):
        kind = "dir"
    else:
        kind = "file"
    size = stat_result.st_size if stat_module.S_ISREG(mode) else None
    return f"{kind}:{size}:{stat_result.st_mtime_ns}:{mode}:0"


def fingerprint_path(path: str) -> str:
    target = Path(path)
    if not target.exists():
        return "missing"
    return fingerprint_from_stat(target, target.lstat())


def collect_touched_paths(journal: AccessJournal, predicted: PredictedEffects) -> set[str]:
    paths: set[str] = set()
    paths.update(journal.metadata_reads)
    paths.update(journal.content_reads)
    paths.update(journal.creates)
    paths.update(journal.deletes)
    paths.update(journal.metadata_writes)
    paths.update(journal.content_writes)
    paths.update(src for src, _dst in journal.renames)
    paths.update(_dst for _src, _dst in journal.renames)
    paths.update(predicted.reads)
    paths.update(predicted.creates)
    paths.update(predicted.updates)
    paths.update(predicted.deletes)
    paths.update(src for src, _dst in predicted.renames)
    paths.update(_dst for _src, _dst in predicted.renames)
    return paths


def fingerprints_for_paths(
    paths: set[str],
    snapshot: WorkspaceSnapshot | None = None,
) -> dict[str, str]:
    fingerprints: dict[str, str] = {}
    for path in sorted(paths):
        if snapshot is not None and (node := snapshot.nodes.get(path)) is not None:
            fingerprints[path] = fingerprint_node(node)
        else:
            fingerprints[path] = fingerprint_path(path)
    return fingerprints
