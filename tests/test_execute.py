from __future__ import annotations as _annotations

from pathlib import Path

import pytest

from vsh.execute import execute_approved
from vsh.plans.store import plan_store
from vsh.schemas import LsCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_execute_approved_rejects_unapproved_plan(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    simulate_command(LsCommand(path="."), snapshot)

    with pytest.raises(KeyError, match="unknown approval token"):
        execute_approved("missing-token")


def test_execute_approved_rejects_record_without_approval(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(
        LsCommand(path=".", all=True, long=True, raw_command="ls -la ."), snapshot
    )
    token = plan_store.approve(result.plan_id)
    record = plan_store.get(result.plan_id)
    record.approval_token = None
    monkeypatch.setattr(plan_store, "get_by_token", lambda _approval_token: record)

    with pytest.raises(ValueError, match="plan not approved"):
        execute_approved(token.token)


def test_execute_approved_returns_staged_result(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(
        LsCommand(path=".", all=True, long=True, raw_command="ls -la ."), snapshot
    )
    token = plan_store.approve(result.plan_id)

    execution = execute_approved(token.token)

    assert execution.plan_id == result.plan_id
    assert execution.applied is True
    assert execution.execution_eligible is True
