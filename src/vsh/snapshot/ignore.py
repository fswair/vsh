from __future__ import annotations as _annotations

import os
from pathlib import Path

import pathspec

from vsh.snapshot.constants import IGNORED_DIRECTORIES

__all__ = ("SnapshotIgnoreMatcher", "build_ignore_matcher")


class SnapshotIgnoreMatcher:
    def __init__(
        self,
        *,
        workspace_root: Path,
        extra_patterns: tuple[str, ...] = (),
    ) -> None:
        patterns: list[str] = [f"{name}/" for name in sorted(IGNORED_DIRECTORIES)]
        gitignore = workspace_root / ".gitignore"
        if gitignore.is_file():
            lines = gitignore.read_text(encoding="utf-8").splitlines()
            patterns.extend(
                line for line in lines if line.strip() and not line.strip().startswith("#")
            )
        env_extra = os.environ.get("VSH_SNAPSHOT_IGNORE", "").strip()
        if env_extra:
            patterns.extend(part.strip() for part in env_extra.split(",") if part.strip())
        patterns.extend(extra_patterns)
        self._root = workspace_root.resolve()
        self._spec = pathspec.PathSpec.from_lines("gitwildmatch", patterns)

    def is_ignored(self, path: Path, *, is_dir: bool) -> bool:
        if any(part in IGNORED_DIRECTORIES for part in path.parts):
            return True
        try:
            relative = path.resolve().relative_to(self._root)
        except ValueError:
            return False
        candidate = str(relative).replace("\\", "/")
        if is_dir and not candidate.endswith("/"):
            candidate = f"{candidate}/"
        return self._spec.match_file(candidate)


def build_ignore_matcher(workspace_root: Path) -> SnapshotIgnoreMatcher:
    return SnapshotIgnoreMatcher(workspace_root=workspace_root)
