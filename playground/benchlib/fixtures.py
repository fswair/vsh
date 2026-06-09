from __future__ import annotations as _annotations

from pathlib import Path

__all__ = ("prepare_scaled_workspace",)


def prepare_scaled_workspace(root: Path, *, file_count: int, file_size: int) -> None:
    root.mkdir(parents=True, exist_ok=True)
    payload = ("needle line\n" * max(1, file_size // 12))[:file_size]
    for index in range(file_count):
        target = root / f"file_{index:04d}.txt"
        target.write_text(
            payload if index % 7 else f"needle special {index}\n{payload}",
            encoding="utf-8",
        )
    (root / "subdir").mkdir(exist_ok=True)
    (root / "subdir" / "nested.txt").write_text("nested needle\n", encoding="utf-8")
