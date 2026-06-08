from __future__ import annotations as _annotations

from pathlib import Path

from vsh.simulate.models import AccessJournal, PredictedEffects
from vsh.snapshot.models import SnapshotNode

__all__ = (
    "collect_touched_paths",
    "fingerprint_node",
    "fingerprint_path",
    "fingerprints_for_paths",
)


def fingerprint_node(node: SnapshotNode) -> str:
    return f"{node.kind}:{node.size}:{node.mtime_ns}:{node.mode}:{node.revision}"


def fingerprint_path(path: str) -> str:
    target = Path(path)
    if not target.exists():
        return "missing"
    stat_result = target.lstat()
    if target.is_symlink():
        kind = "symlink"
    elif target.is_dir():
        kind = "dir"
    else:
        kind = "file"
    size = stat_result.st_size if target.is_file() else None
    return f"{kind}:{size}:{stat_result.st_mtime_ns}:{stat_result.st_mode}:0"


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


def fingerprints_for_paths(paths: set[str]) -> dict[str, str]:
    return {path: fingerprint_path(path) for path in sorted(paths)}
