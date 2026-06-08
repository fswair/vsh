from __future__ import annotations as _annotations

from pathlib import Path

import pytest

from vsh.runtime import RuntimeLedger, runtime
from vsh.schemas import PwdCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_runtime_get_snapshot_requires_recorded_snapshot() -> None:
    ledger = RuntimeLedger()

    with pytest.raises(KeyError, match="no snapshot has been recorded"):
        ledger.get_snapshot()


def test_runtime_get_plan_returns_recorded_plan(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(PwdCommand(), snapshot)
    record = runtime.get_plan(result.plan_id)

    assert record.plan_id == result.plan_id
