from __future__ import annotations as _annotations

from pathlib import Path

import pytest
from conftest import with_execution_reason

from vsh.mcp import resources, tools
from vsh.plans import approve_plan
from vsh.plans.store import plan_store
from vsh.schemas import (
    CatCommand,
    CdCommand,
    ChmodCommand,
    DuCommand,
    EchoCommand,
    FindCommand,
    GrepCommand,
    LnCommand,
    LsCommand,
    PwdCommand,
    RemoveCommand,
    SedCommand,
    StatCommand,
)
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_snapshot_workspace_builds_graph(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    nested = workspace / "src"
    nested.mkdir()
    (nested / "main.py").write_text("print('hi')\n", encoding="utf-8")

    snapshot = snapshot_workspace(str(workspace))

    assert snapshot.session.workspace_root == str(workspace.resolve())
    assert str(workspace.resolve()) in snapshot.nodes
    assert str((workspace / "src").resolve()) in snapshot.nodes
    assert str((workspace / "src" / "main.py").resolve()) in snapshot.nodes


def test_simulate_command_and_approval_flow(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "src").mkdir()

    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(CdCommand(path="src"), snapshot)
    token = approve_plan(result.plan_id)
    execution = tools.execute_approved(token.token)

    assert isinstance(result.command, CdCommand)
    assert result.command.path == "src"
    assert result.decision == "approve"
    assert result.execution_eligible is True
    assert result.raw_matches_shell_preview is None
    cwd_after = result.predicted_effects.cwd_after
    assert cwd_after is not None
    assert cwd_after.endswith("/src")
    assert token.plan_id == result.plan_id
    assert execution["plan_id"] == result.plan_id
    assert execution["execution_eligible"] is True
    assert execution["applied"] is True
    assert plan_store.get(result.plan_id).approval_token is not None


def test_tools_and_resources_use_current_snapshot(tmp_path: Path, monkeypatch) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "README.md").write_text("hello\n", encoding="utf-8")
    monkeypatch.chdir(workspace)

    snapshot_payload = tools.snapshot_workspace()
    projection_payload = resources.workspace_projection_current()
    spec_payload = resources.command_spec("vsh_list")

    assert snapshot_payload["session"]["workspace_root"] == str(workspace.resolve())
    assert "node_count" in snapshot_payload
    assert "nodes" not in snapshot_payload
    assert projection_payload["workspace_root"] == str(workspace.resolve())
    assert spec_payload["spec"]["name"] == "vsh_list"


def test_pwd_and_ls_simulation(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    pwd_result = simulate_command(PwdCommand(), snapshot)
    ls_result = simulate_command(LsCommand(path=".", all=True), snapshot)

    assert pwd_result.shell_preview == "pwd"
    assert pwd_result.predicted_effects.cwd_after == snapshot.session.cwd_logical
    assert ls_result.command.kind == "list"


def test_common_agent_read_commands_simulate_reads(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    src = workspace / "src"
    src.mkdir()
    readme = workspace / "README.md"
    main = src / "main.py"
    readme.write_text("hello\n", encoding="utf-8")
    main.write_text("print('hi')\n", encoding="utf-8")

    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    cat_result = simulate_command(CatCommand(path="README.md"), snapshot)
    sed_result = simulate_command(SedCommand(script="1,20p", path="README.md"), snapshot)
    grep_result = simulate_command(
        GrepCommand(pattern="print", path="src", recursive=True), snapshot
    )
    find_result = simulate_command(FindCommand(path="src", name="*.py", type="file"), snapshot)
    stat_result = simulate_command(StatCommand(path="README.md"), snapshot)
    du_result = simulate_command(DuCommand(path="src", summarize=True), snapshot)

    assert cat_result.decision == "approve"
    assert str(readme.resolve()) in cat_result.journal.content_reads
    assert sed_result.decision == "approve"
    assert str(readme.resolve()) in sed_result.journal.content_reads
    assert grep_result.decision == "approve"
    assert str(main.resolve()) in grep_result.journal.content_reads
    assert find_result.decision == "approve"
    assert str(main.resolve()) in find_result.journal.metadata_reads
    assert stat_result.decision == "approve"
    assert str(readme.resolve()) in stat_result.journal.metadata_reads
    assert du_result.decision == "approve"
    assert str(main.resolve()) in du_result.journal.metadata_reads


def test_common_agent_read_commands_reject_workspace_escape(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    result = simulate_command(CatCommand(path="../outside.txt"), snapshot)

    assert result.decision == "reject"
    assert result.reason == "target path escapes workspace root"


def test_common_agent_write_metadata_and_link_commands_simulate_effects(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    script = workspace / "script.sh"
    script.write_text("#!/bin/sh\n", encoding="utf-8")

    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    stdout_echo = simulate_command(EchoCommand(text="hello"), snapshot)
    file_echo = simulate_command(
        with_execution_reason(EchoCommand(text="hello", output_path="notes.txt")),
        snapshot,
    )
    chmod_result = simulate_command(
        with_execution_reason(ChmodCommand(mode="+x", path="script.sh")),
        snapshot,
    )
    link_result = simulate_command(
        with_execution_reason(LnCommand(src="script.sh", dst="run.sh", symbolic=True)),
        snapshot,
    )

    assert stdout_echo.decision == "approve"
    assert stdout_echo.execution_eligible is True
    assert file_echo.decision == "approve_with_warning"
    assert str((workspace / "notes.txt").resolve()) in file_echo.predicted_effects.creates
    assert chmod_result.decision == "approve_with_warning"
    assert str(script.resolve()) in chmod_result.predicted_effects.updates
    assert link_result.decision == "approve_with_warning"
    assert str((workspace / "run.sh").resolve()) in link_result.predicted_effects.creates


def test_sed_in_place_multi_file_simulation_reports_all_changed_files(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    first = workspace / "first.txt"
    second = workspace / "second.txt"
    first.write_text("old\n", encoding="utf-8")
    second.write_text("old\n", encoding="utf-8")

    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(
        with_execution_reason(
            SedCommand(script="s/old/new/g", paths=["first.txt", "second.txt"], in_place=True),
        ),
        snapshot,
    )

    assert result.decision == "approve_with_warning"
    assert str(first.resolve()) in result.predicted_effects.updates
    assert str(second.resolve()) in result.predicted_effects.updates
    assert str(first.resolve()) in result.journal.content_writes
    assert str(second.resolve()) in result.journal.content_writes


def test_simulate_command_marks_matching_raw_command_execution_eligible(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    result = simulate_command(
        LsCommand(path=".", all=True, long=True, raw_command="ls -la ."), snapshot
    )

    assert result.decision == "approve"
    assert result.raw_matches_shell_preview is True
    assert result.execution_eligible is True
    assert result.execution_eligibility_reason is None


def test_simulation_result_dump_preserves_concrete_command_fields(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    result = simulate_command(
        LsCommand(path=".", all=True, long=True, raw_command="ls -la ."), snapshot
    )
    command_payload = result.model_dump()["command"]

    assert command_payload["path"] == "."
    assert command_payload["all"] is True
    assert command_payload["long"] is True
    assert command_payload["raw_command"] == "ls -la ."


def test_simulate_command_marks_mismatched_raw_command_execution_ineligible(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    result = simulate_command(
        LsCommand(path=".", all=True, long=True, raw_command="ls -al ."), snapshot
    )

    assert result.decision == "approve"
    assert result.raw_matches_shell_preview is False
    assert result.execution_eligible is False
    assert (
        result.execution_eligibility_reason
        == "raw command does not match the canonical shell preview"
    )


def test_execute_approved_rejects_execution_ineligible_plan(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    result = simulate_command(
        LsCommand(path=".", all=True, long=True, raw_command="ls -al ."), snapshot
    )
    with pytest.raises(ValueError, match="not eligible for approval"):
        approve_plan(result.plan_id)


def test_remove_command_rejects_home_shorthand(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(RemoveCommand(path="~/", recursive=True, force=True), snapshot)

    assert result.decision == "reject"
    assert result.reason == "destructive command cannot target the home directory shorthand"


def test_remove_command_rejects_workspace_root_delete(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(
        RemoveCommand(path=str(workspace), recursive=True, force=True), snapshot
    )

    assert result.decision == "reject"
    assert (
        result.reason
        == "destructive command cannot target the workspace root or one of its ancestors"
    )


def test_snapshot_workspace_rejects_protected_root() -> None:
    with pytest.raises(ValueError, match="workspace root is too broad or protected"):
        snapshot_workspace(str(Path.home()))
