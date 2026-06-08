from __future__ import annotations as _annotations

from pathlib import Path
from typing import ClassVar

from vsh.plans.models import SimulationResult
from vsh.schemas import (
    CatCommand,
    CdCommand,
    ChmodCommand,
    CopyCommand,
    DuCommand,
    EchoCommand,
    FindCommand,
    GrepCommand,
    HeadCommand,
    LnCommand,
    LsCommand,
    MkdirCommand,
    MoveCommand,
    NlCommand,
    PwdCommand,
    RemoveCommand,
    RgCommand,
    SedCommand,
    SideEffect,
    StatCommand,
    StructuredCommand,
    TailCommand,
    TouchCommand,
    WcCommand,
)
from vsh.schemas.common import CommandKind
from vsh.simulate.engine import (
    _evaluate_execution_eligibility,
    _first_outside_workspace,
    _read_scope,
    simulate_command,
)
from vsh.snapshot.builder import snapshot_workspace
from vsh.snapshot.models import WorkspaceSnapshot


class _FallbackMutationCommand(StructuredCommand):
    _command_alias: ClassVar[str] = "fallback"
    kind: CommandKind = "write"
    side_effects: list[SideEffect] = [SideEffect(kind="write", risks=[])]


def _snapshot(workspace: Path, *, cwd: str | None = None) -> WorkspaceSnapshot:
    return snapshot_workspace(str(workspace), cwd=cwd or str(workspace))


def test_cd_rejects_workspace_escape(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    result = simulate_command(CdCommand(path="../outside"), _snapshot(workspace))

    assert result.decision == "reject"
    assert result.reason == "target path escapes workspace root"


def test_ls_lists_known_and_unknown_directories(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = _snapshot(workspace)
    missing = simulate_command(LsCommand(path="missing"), snapshot)
    current = simulate_command(LsCommand(path="."), snapshot)

    assert missing.decision == "approve"
    assert current.decision == "approve"
    assert len(current.predicted_effects.reads) >= 1


def test_ls_rejects_workspace_escape(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = _snapshot(workspace)

    for path in ("/", "..", "../outside"):
        result = simulate_command(LsCommand(path=path), snapshot)
        assert result.decision == "reject"
        assert result.reason == "target path escapes workspace root"
        assert result.execution_eligible is False


def test_read_commands_reject_outside_targets(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = _snapshot(workspace)

    for command in (
        HeadCommand(path="../outside.txt"),
        TailCommand(path="../outside.txt"),
        NlCommand(path="../outside.txt"),
        WcCommand(path="../outside.txt"),
        DuCommand(path="../outside"),
        StatCommand(path="../outside.txt"),
        GrepCommand(pattern="x", path="../outside"),
        RgCommand(pattern="x", path="../outside"),
        FindCommand(path="../outside"),
    ):
        result = simulate_command(command, snapshot)
        assert result.decision == "reject"
        assert result.reason == "target path escapes workspace root"


def test_sed_read_mode_rejects_outside_paths(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = _snapshot(workspace)

    result = simulate_command(SedCommand(script="1p", path="../outside.txt"), snapshot)

    assert result.decision == "reject"


def test_mutation_commands_predict_overlay_effects(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "src.txt").write_text("data\n", encoding="utf-8")
    snapshot = _snapshot(workspace)

    cases: list[tuple[StructuredCommand, str]] = [
        (MkdirCommand(path="build", parents=True), "approve_with_warning"),
        (TouchCommand(path="new.txt"), "approve_with_warning"),
        (TouchCommand(path="new.txt", no_create=True), "approve_with_warning"),
        (MoveCommand(src="src.txt", dst="dst.txt", overwrite=True), "approve_with_warning"),
        (CopyCommand(src="src.txt", dst="copy.txt", recursive=True), "approve_with_warning"),
        (RemoveCommand(path="src.txt"), "approve_with_warning"),
        (EchoCommand(text="x", output_path="out.txt"), "approve_with_warning"),
        (EchoCommand(text="x", output_path="out.txt", append=True), "approve_with_warning"),
        (EchoCommand(text="x", output_path=""), "approve_with_warning"),
        (ChmodCommand(mode="+x", path="src.txt", recursive=True), "approve_with_warning"),
        (
            LnCommand(src="src.txt", dst="link.txt", symbolic=True, force=True),
            "approve_with_warning",
        ),
    ]
    for command, decision in cases:
        result = simulate_command(command, snapshot)
        assert result.decision == decision


def test_fallback_mutation_command_uses_generic_overlay(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = _snapshot(workspace)

    result = simulate_command(_FallbackMutationCommand(), snapshot)

    assert result.decision == "approve_with_warning"


def test_read_scope_handles_missing_file_and_directory_prefixes(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    src = workspace / "src"
    src.mkdir()
    (src / "main.py").write_text("print('hi')\n", encoding="utf-8")
    snapshot = _snapshot(workspace)
    src_path = str(src.resolve())
    file_path = str((src / "main.py").resolve())
    missing_path = str((workspace / "missing").resolve())

    assert _read_scope(snapshot, missing_path) == [missing_path]
    assert _read_scope(snapshot, file_path) == [file_path]
    scoped = _read_scope(snapshot, src_path)
    assert file_path in scoped
    assert src_path in scoped


def test_read_scope_uses_tree_traversal_for_large_subtrees(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    import vsh.simulate.engine as engine_module

    workspace = tmp_path / "workspace"
    workspace.mkdir()
    nested = workspace / "pkg"
    nested.mkdir()
    for index in range(3):
        (nested / f"file{index}.txt").write_text("x\n", encoding="utf-8")
    snapshot = _snapshot(workspace)
    monkeypatch.setattr(engine_module, "_READ_SCOPE_TREE_MIN_NODES", 1)

    scoped = _read_scope(snapshot, str(nested.resolve()))

    assert str(nested.resolve()) in scoped
    assert str((nested / "file0.txt").resolve()) in scoped


def test_first_outside_workspace_returns_first_offender(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = _snapshot(workspace)
    inside = str((workspace / "inside.txt").resolve())
    outside = str((tmp_path / "outside.txt").resolve())

    assert _first_outside_workspace([inside, outside], snapshot) == outside


def test_execution_eligibility_rejects_reject_decisions() -> None:
    eligible, reason = _evaluate_execution_eligibility(
        decision="reject",
        raw_matches_shell_preview=True,
    )

    assert eligible is False
    assert reason is not None


def test_execution_eligibility_allows_warned_mutations() -> None:
    eligible, reason = _evaluate_execution_eligibility(
        decision="approve_with_warning",
        raw_matches_shell_preview=True,
    )

    assert eligible is True
    assert reason is None


def test_cat_read_simulation_approves_existing_file(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    readme = workspace / "README.md"
    readme.write_text("hello\n", encoding="utf-8")
    snapshot = _snapshot(workspace)

    result = simulate_command(CatCommand(path="README.md"), snapshot)

    assert result.decision == "approve"
    assert str(readme.resolve()) in result.journal.content_reads


def test_simulation_result_is_typed(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    result = simulate_command(PwdCommand(), _snapshot(workspace))

    assert isinstance(result, SimulationResult)
