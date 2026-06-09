from __future__ import annotations as _annotations

from pathlib import Path

import pytest
from conftest import with_execution_reason

from vsh.execute.dispatch import ExecutionContext, apply_command
from vsh.plans.approval import approve_plan
from vsh.schemas import (
    CatCommand,
    CopyCommand,
    EchoCommand,
    LsCommand,
    MkdirCommand,
    MoveCommand,
    RemoveCommand,
    SedCommand,
    TouchCommand,
)
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_apply_command_mkdir_and_touch_mutate_workspace(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))

    mkdir_effects = apply_command(MkdirCommand(path="build", parents=True), ctx)
    touch_effects = apply_command(TouchCommand(path="build/app.txt"), ctx)

    assert (workspace / "build").is_dir()
    assert (workspace / "build" / "app.txt").is_file()
    assert str((workspace / "build").resolve()) in mkdir_effects.creates
    assert str((workspace / "build" / "app.txt").resolve()) in touch_effects.creates


def test_apply_command_move_copy_and_remove(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    source = workspace / "a.txt"
    source.write_text("data\n", encoding="utf-8")
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))

    apply_command(MoveCommand(src="a.txt", dst="b.txt"), ctx)
    assert not source.exists()
    assert (workspace / "b.txt").read_text(encoding="utf-8") == "data\n"

    apply_command(CopyCommand(src="b.txt", dst="copy.txt"), ctx)
    assert (workspace / "copy.txt").exists()

    apply_command(RemoveCommand(path="copy.txt"), ctx)
    assert not (workspace / "copy.txt").exists()


def test_apply_command_echo_and_sed_in_place(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "notes.txt"
    target.write_text("old\n", encoding="utf-8")
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))

    apply_command(EchoCommand(text="hello", output_path="out.txt"), ctx)
    apply_command(
        SedCommand(script="s/old/new/g", paths=["notes.txt"], in_place=True),
        ctx,
    )

    assert (workspace / "out.txt").read_text(encoding="utf-8") == "hello\n"
    assert target.read_text(encoding="utf-8") == "new\n"


def test_execute_approved_applies_mkdir_plan(tmp_path: Path) -> None:
    from vsh.execute import execute_approved

    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(
        with_execution_reason(MkdirCommand(path="dist", parents=True)),
        snapshot,
    )
    token = approve_plan(result.plan_id)

    execution = execute_approved(token.token)

    assert execution.applied is True
    assert execution.matches_prediction is True
    assert (workspace / "dist").is_dir()


def test_execute_approved_rejects_stale_plan(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from vsh.execute import execute_approved

    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "tracked.txt"
    target.write_text("v1\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(
        with_execution_reason(TouchCommand(path="tracked.txt", no_create=True)),
        snapshot,
    )
    token = approve_plan(result.plan_id)
    target.write_text("v2\n", encoding="utf-8")

    execution = execute_approved(token.token)

    assert execution.applied is False
    assert execution.revalidation.status == "stale"
    assert execution.reason is not None
    assert "stale" in execution.reason


def test_apply_command_rejects_workspace_escape_at_execution_layer(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))

    with pytest.raises(ValueError, match="path escapes workspace root"):
        apply_command(LsCommand(path=".."), ctx)

    with pytest.raises(ValueError, match="path escapes workspace root"):
        apply_command(CatCommand(path="../outside.txt"), ctx)
