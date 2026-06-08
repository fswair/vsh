from __future__ import annotations as _annotations

import pytest


@pytest.fixture(autouse=True)
def disable_disk_persistence(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("VSH_PERSIST", "0")
