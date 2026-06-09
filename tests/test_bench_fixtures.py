from __future__ import annotations as _annotations

import sys
from pathlib import Path

PLAYGROUND = Path(__file__).resolve().parent.parent / "playground"
if str(PLAYGROUND) not in sys.path:
    sys.path.insert(0, str(PLAYGROUND))

from benchlib.fixtures import prepare_scaled_workspace  # noqa: E402


def test_prepare_scaled_workspace_creates_expected_files(tmp_path: Path) -> None:
    root = tmp_path / "scaled"
    prepare_scaled_workspace(root, file_count=12, file_size=64)
    files = sorted(root.glob("file_*.txt"))
    assert len(files) == 12
    assert (root / "subdir" / "nested.txt").is_file()


def test_prepare_scaled_workspace_supports_1k_scale(tmp_path: Path) -> None:
    root = tmp_path / "large"
    prepare_scaled_workspace(root, file_count=1000, file_size=32)
    assert len(list(root.glob("file_*.txt"))) == 1000
