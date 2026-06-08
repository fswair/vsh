from __future__ import annotations as _annotations

from pathlib import Path

import pytest

from vsh.plans.approval import approve_plan, auto_approve_plan
from vsh.schemas import LsCommand, RemoveCommand, TouchCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_read_only_plan_is_auto_approvable(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(LsCommand(path="."), snapshot)

    assert result.approval_tier == "read_only"
    assert result.requires_manual_approval is False

    token = auto_approve_plan(result.plan_id)
    assert token.plan_id == result.plan_id


def test_mutation_plan_requires_manual_approval(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(TouchCommand(path="new.txt"), snapshot)

    assert result.approval_tier == "mutation"
    assert result.requires_manual_approval is True

    with pytest.raises(ValueError, match="requires manual approval"):
        auto_approve_plan(result.plan_id)

    token = approve_plan(result.plan_id)
    assert token.plan_id == result.plan_id


def test_destructive_plan_requires_manual_approval(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    doomed = workspace / "old.txt"
    doomed.write_text("x\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(RemoveCommand(path="old.txt"), snapshot)

    assert result.approval_tier == "destructive"
    assert result.requires_manual_approval is True
