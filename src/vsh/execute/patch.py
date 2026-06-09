from __future__ import annotations as _annotations

from pathlib import Path

from vsh.schemas import ApplyPatchCommand

__all__ = ("apply_patch_to_file",)


def apply_patch_to_file(command: ApplyPatchCommand, target: str) -> str:
    path = Path(target)
    original = path.read_text(encoding="utf-8") if path.exists() else ""
    if "\n===\n" not in command.patch:
        msg = "patch must use 'old\\n===\\nnew' search-replace format"
        raise ValueError(msg)
    old, new = command.patch.split("\n===\n", 1)
    if old not in original:
        msg = "search-replace old block not found in target file"
        raise ValueError(msg)
    updated = original.replace(old, new, 1)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(updated, encoding="utf-8")
    return updated
