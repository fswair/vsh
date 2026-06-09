from __future__ import annotations as _annotations

from pathlib import Path

from conftest import MUTATION_REASON, with_execution_reason

from vsh.schemas import LsCommand, TouchCommand
from vsh.simulate.engine import _has_execution_reason, simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_has_execution_reason() -> None:
    assert _has_execution_reason(TouchCommand(path="x", execution_reason="because")) is True
    assert _has_execution_reason(TouchCommand(path="x", execution_reason="  ")) is False
    assert _has_execution_reason(TouchCommand(path="x")) is False


def test_mutation_without_execution_reason_is_rejected(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(TouchCommand(path="new.txt"), snapshot)
    assert result.decision == "reject"
    assert result.reason == "execution_reason is required for mutation commands"
    assert result.execution_eligible is False


def test_mutation_with_execution_reason_simulates(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(
        with_execution_reason(TouchCommand(path="new.txt")),
        snapshot,
    )
    assert result.decision == "approve_with_warning"
    assert result.command.execution_reason == MUTATION_REASON


def test_read_only_command_does_not_require_execution_reason(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(LsCommand(path="."), snapshot)
    assert result.decision == "approve"
