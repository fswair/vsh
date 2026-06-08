from __future__ import annotations as _annotations

from pathlib import Path

from vsh.schemas import CopyCommand, MoveCommand, RemoveCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_remove_rejects_dangerous_shorthand_paths(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    for path in (".", "./", "..", "../"):
        result = simulate_command(RemoveCommand(path=path, recursive=True, force=True), snapshot)
        assert result.decision == "reject"
        assert "shorthand" in (result.reason or "")


def test_remove_rejects_protected_home_parent_targets(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    home_parent = str(Path.home().parent)

    result = simulate_command(
        RemoveCommand(path=home_parent, recursive=True, force=True),
        snapshot,
    )

    assert result.decision == "reject"
    assert result.reason is not None


def test_mutation_outside_workspace_is_rejected(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    outside = tmp_path / "outside.txt"
    outside.write_text("x\n", encoding="utf-8")

    result = simulate_command(CopyCommand(src="src.txt", dst=str(outside)), snapshot)

    assert result.decision == "reject"
    assert result.reason is not None
    assert "escapes workspace root" in result.reason


def test_move_rename_of_workspace_child_is_warned(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    source = workspace / "a.txt"
    source.write_text("x\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    result = simulate_command(MoveCommand(src="a.txt", dst="b.txt"), snapshot)

    assert result.decision == "approve_with_warning"
