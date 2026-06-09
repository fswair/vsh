from __future__ import annotations as _annotations

import os
from functools import lru_cache

from .filesystem import FileArtifactStore, default_filesystem_root
from .memory import MemoryArtifactStore
from .store import ArtifactStore

__all__ = ("artifact_spill_bytes", "create_artifact_store")


def artifact_spill_bytes() -> int:
    raw = os.environ.get("VSH_ARTIFACT_SPILL_BYTES", "8192")
    try:
        return max(1, int(raw))
    except ValueError:
        return 8192


def _persistence_enabled() -> bool:
    return os.environ.get("VSH_PERSIST", "1") != "0"


def create_artifact_store() -> ArtifactStore:
    store_kind = os.environ.get("VSH_ARTIFACT_STORE", "filesystem").strip().lower()
    if store_kind == "memory" or not _persistence_enabled():
        return MemoryArtifactStore()
    return FileArtifactStore(root=default_filesystem_root())


@lru_cache(maxsize=1)
def default_artifact_store() -> ArtifactStore:
    return create_artifact_store()
