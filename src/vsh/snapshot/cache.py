from __future__ import annotations as _annotations

import os
import time
from dataclasses import dataclass, field
from pathlib import Path

from vsh.snapshot.models import WorkspaceSnapshot

__all__ = ("SnapshotCache", "snapshot_cache")


@dataclass
class SnapshotCache:
    _entries: dict[str, tuple[int, WorkspaceSnapshot]] = field(default_factory=dict)

    def workspace_fingerprint(self, root: Path) -> int:
        try:
            stat = root.stat()
        except OSError:
            return 0
        return hash((stat.st_dev, stat.st_ino, stat.st_mtime_ns))

    def get(self, workspace_root: str) -> WorkspaceSnapshot | None:
        root = Path(workspace_root)
        key = str(root.resolve())
        entry = self._entries.get(key)
        if entry is None:
            return None
        fingerprint, snapshot = entry
        if fingerprint != self.workspace_fingerprint(root):
            return None
        return snapshot

    def put(self, snapshot: WorkspaceSnapshot) -> None:
        root = Path(snapshot.session.workspace_root)
        key = str(root.resolve())
        self._entries[key] = (self.workspace_fingerprint(root), snapshot)

    def clear(self) -> None:
        self._entries.clear()


snapshot_cache = SnapshotCache()


def cache_enabled() -> bool:
    return os.environ.get("VSH_SNAPSHOT_CACHE", "1") != "0"


def snapshot_age_seconds(snapshot: WorkspaceSnapshot) -> float:
    return max(0.0, (time.time_ns() - snapshot.generated_at_ns) / 1_000_000_000)
