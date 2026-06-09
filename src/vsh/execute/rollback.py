from __future__ import annotations as _annotations

import shutil
import time
from dataclasses import dataclass, field
from pathlib import Path

__all__ = ("RollbackSession", "backup_file", "restore_session")


@dataclass
class RollbackSession:
    backup_root: Path
    entries: list[tuple[Path, Path | None]] = field(default_factory=list)

    def record(self, target: Path, existed: bool) -> None:
        if not existed:
            self.entries.append((target, None))
            return
        backup_path = self.backup_root / f"{int(time.time_ns())}_{target.name}"
        backup_path.parent.mkdir(parents=True, exist_ok=True)
        if target.is_dir():
            shutil.copytree(target, backup_path)
        else:
            shutil.copy2(target, backup_path)
        self.entries.append((target, backup_path))


def backup_file(session: RollbackSession, target: Path) -> None:
    session.record(target, target.exists())


def restore_session(session: RollbackSession) -> None:
    for target, backup in reversed(session.entries):
        if backup is None:
            if target.is_dir():
                shutil.rmtree(target, ignore_errors=True)
            elif target.exists():
                target.unlink()
            continue
        if target.exists():
            if target.is_dir():
                shutil.rmtree(target)
            else:
                target.unlink()
        if backup.is_dir():
            shutil.copytree(backup, target)
        else:
            shutil.copy2(backup, target)
    shutil.rmtree(session.backup_root, ignore_errors=True)
