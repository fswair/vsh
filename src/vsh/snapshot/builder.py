from __future__ import annotations as _annotations

import hashlib
import os
import stat as stat_module
import time
import uuid
from pathlib import Path

from vsh.runtime import runtime
from vsh.session import SessionState
from vsh.snapshot.cache import cache_enabled, snapshot_age_seconds, snapshot_cache
from vsh.snapshot.constants import IGNORED_DIRECTORIES
from vsh.snapshot.ignore import build_ignore_matcher

from .models import SnapshotNode, WorkspaceSnapshot

__all__ = (
    "IGNORED_DIRECTORIES",
    "content_hash_enabled",
    "node_for_path",
    "snapshot_node_from_lstat",
    "snapshot_workspace",
)


def content_hash_enabled() -> bool:
    return os.environ.get("VSH_CONTENT_HASH", "0") == "1"


def snapshot_workspace(workspace_root: str, cwd: str | None = None) -> WorkspaceSnapshot:
    session = SessionState.from_workspace_root(workspace_root, cwd=cwd)
    root = Path(session.workspace_root)
    if cache_enabled():
        cached = snapshot_cache.get(str(root))
        if cached is not None and snapshot_age_seconds(cached) < _cache_max_age_seconds():
            runtime.record_snapshot(cached)
            return cached
    nodes = _build_nodes(root)
    snapshot = WorkspaceSnapshot(
        snapshot_id=f"snap_{uuid.uuid4().hex[:12]}",
        session=session,
        generated_at_ns=time.time_ns(),
        nodes=nodes,
    )
    runtime.record_snapshot(snapshot)
    if cache_enabled():
        snapshot_cache.put(snapshot)
    return snapshot


def _cache_max_age_seconds() -> float:
    raw = os.environ.get("VSH_SNAPSHOT_CACHE_MAX_AGE_SECS", "30")
    try:
        return max(0.0, float(raw))
    except ValueError:
        return 30.0


def _build_nodes(root: Path) -> dict[str, SnapshotNode]:
    matcher = build_ignore_matcher(root)
    nodes: dict[str, SnapshotNode] = {}
    stack: list[Path] = [root]
    while stack:
        current_path = stack.pop()
        if current_path != root and matcher.is_ignored(current_path, is_dir=current_path.is_dir()):
            continue
        node = node_for_path(current_path)
        nodes[str(current_path)] = node
        if not current_path.is_dir():
            continue
        try:
            entries = sorted(current_path.iterdir(), key=lambda item: item.name)
        except OSError:
            continue
        child_dirs: list[Path] = []
        for entry in entries:
            if entry.is_dir():
                if matcher.is_ignored(entry, is_dir=True):
                    continue
                child_dirs.append(entry)
                node.children.append(str(entry))
            elif entry.is_file():
                if matcher.is_ignored(entry, is_dir=False):
                    continue
                nodes[str(entry)] = node_for_path(entry)
                node.children.append(str(entry))
        stack.extend(reversed(child_dirs))
    if str(root) not in nodes:
        nodes[str(root)] = node_for_path(root)  # pragma: no cover
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
        if content_hash_enabled():
            content_ref = _content_hash_ref(path)
        else:
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


def _content_hash_ref(path: Path) -> str:
    digest = hashlib.blake2b(path.read_bytes(), digest_size=16).hexdigest()
    return f"hash:blake2b:{digest}"
