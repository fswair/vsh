from __future__ import annotations as _annotations

from pathlib import Path
from typing import ClassVar

import pytest

from vsh.effects import ActualEffects
from vsh.execute.dispatch import (
    ExecutionContext,
    _apply_sed_script,
    _parse_mode,
    apply_command,
    effects_match_prediction,
)
from vsh.execute.realfs import _apply_session_updates, _run_extension_hooks, execute_approved
from vsh.execute.revalidate import revalidate_plan
from vsh.extensions.registry import extensions
from vsh.persistence import PersistenceStore, persistence_store
from vsh.persistence.store import _restore_command
from vsh.plans.approval import approve_plan
from vsh.plans.store import plan_store
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
    SortCommand,
    StatCommand,
    StructuredCommand,
    TailCommand,
    TouchCommand,
    WcCommand,
)
from vsh.simulate.engine import simulate_command
from vsh.simulate.models import PredictedEffects
from vsh.snapshot.builder import snapshot_workspace
from vsh.snapshot.fingerprint import fingerprint_node
from vsh.snapshot.models import SnapshotNode, WorkspaceSnapshot
from vsh.snapshot.refresh import refresh_snapshot_paths


def test_dispatch_success_matrix(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "a.txt").write_text("line\n", encoding="utf-8")
    src = workspace / "src"
    src.mkdir()
    (src / "nested.txt").write_text("nested\n", encoding="utf-8")
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))

    apply_command(PwdCommand(), ctx)
    apply_command(CdCommand(path="src"), ctx)
    ctx.cwd_logical = str(src.resolve())
    apply_command(LsCommand(path="."), ctx)
    apply_command(CatCommand(path="nested.txt"), ctx)
    apply_command(HeadCommand(path="nested.txt"), ctx)
    apply_command(TailCommand(path="nested.txt"), ctx)
    apply_command(NlCommand(path="nested.txt"), ctx)
    apply_command(WcCommand(path="nested.txt", lines=True), ctx)
    apply_command(SortCommand(path="nested.txt"), ctx)
    apply_command(StatCommand(path="nested.txt"), ctx)
    apply_command(DuCommand(path=".", summarize=True), ctx)
    apply_command(GrepCommand(pattern="nested", path="."), ctx)
    apply_command(RgCommand(pattern="nested", path="."), ctx)
    apply_command(FindCommand(path=".", name="*.txt", type="file"), ctx)
    apply_command(EchoCommand(text="hi"), ctx)
    apply_command(MkdirCommand(path="build", parents=True), ctx)
    apply_command(TouchCommand(path="build/new.txt"), ctx)
    apply_command(TouchCommand(path="build/new.txt", no_create=True), ctx)
    apply_command(MoveCommand(src="nested.txt", dst="moved.txt"), ctx)
    apply_command(CopyCommand(src="moved.txt", dst="copy.txt"), ctx)
    apply_command(
        EchoCommand(text="saved", output_path="out.txt", append=False),
        ctx,
    )
    apply_command(
        EchoCommand(text="more", output_path="out.txt", append=True),
        ctx,
    )
    apply_command(ChmodCommand(mode="+x", path="moved.txt"), ctx)
    apply_command(LnCommand(src="moved.txt", dst="link.txt", symbolic=True, force=True), ctx)
    apply_command(
        SedCommand(script="s/nested/updated/g", paths=["moved.txt"], in_place=True),
        ctx,
    )
    apply_command(RemoveCommand(path="copy.txt"), ctx)
    ctx.cwd_logical = str(workspace.resolve())
    nested_dir = workspace / "drop"
    nested_dir.mkdir()
    (nested_dir / "x.txt").write_text("x\n", encoding="utf-8")
    apply_command(RemoveCommand(path="drop", recursive=True), ctx)
    apply_command(CopyCommand(src="src", dst="src-copy", recursive=True, overwrite=True), ctx)
    apply_command(LnCommand(src="a.txt", dst="hardlink", symbolic=False, force=True), ctx)
    apply_command(SedCommand(script="1p", path="a.txt", in_place=False), ctx)
    apply_command(
        EchoCommand(text="plain", output_path="plain.txt", no_newline=True),
        ctx,
    )
    apply_command(ChmodCommand(mode="755", path="src-copy", recursive=True), ctx)


def test_dispatch_error_paths(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "a.txt").write_text("x\n", encoding="utf-8")
    (workspace / "b.txt").write_text("y\n", encoding="utf-8")
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))

    with pytest.raises(ValueError, match="not a directory"):
        apply_command(CdCommand(path="missing"), ctx)
    with pytest.raises(ValueError, match="does not exist"):
        apply_command(TouchCommand(path="missing.txt", no_create=True), ctx)
    with pytest.raises(FileNotFoundError):
        apply_command(MoveCommand(src="missing.txt", dst="z.txt"), ctx)
    with pytest.raises(ValueError, match="destination already exists"):
        apply_command(MoveCommand(src="a.txt", dst="b.txt"), ctx)
    with pytest.raises(ValueError, match="directory but recursive is false"):
        apply_command(CopyCommand(src=".", dst="copy"), ctx)
    with pytest.raises(ValueError, match="destination already exists"):
        apply_command(CopyCommand(src="a.txt", dst="b.txt"), ctx)
    with pytest.raises(ValueError, match="directory but recursive is false"):
        apply_command(RemoveCommand(path="."), ctx)
    with pytest.raises(ValueError, match="unsupported chmod mode"):
        apply_command(ChmodCommand(mode="g+x", path="a.txt"), ctx)
    with pytest.raises(ValueError, match="destination already exists"):
        apply_command(LnCommand(src="a.txt", dst="b.txt"), ctx)
    with pytest.raises(ValueError, match="unsupported sed script"):
        apply_command(SedCommand(script="1,2p", paths=["a.txt"], in_place=True), ctx)

    class _UnknownCommand(StructuredCommand):
        _command_alias: ClassVar[str] = "unknown"

    with pytest.raises(ValueError, match="unsupported command"):
        apply_command(_UnknownCommand(), ctx)


def test_parse_mode_and_sed_helpers(tmp_path: Path) -> None:
    target = tmp_path / "file.txt"
    target.write_text("old\n", encoding="utf-8")
    _apply_sed_script(str(target), "s/old/new/g", ".bak")
    assert target.read_text(encoding="utf-8") == "new\n"
    assert (tmp_path / "file.txt.bak").read_text(encoding="utf-8") == "old\n"
    assert _parse_mode("644", 0o600) == 0o644
    assert _parse_mode("+x", 0o600) & 0o111


def test_effects_match_prediction_helper() -> None:
    predicted = PredictedEffects(creates=["/tmp/a"], cwd_after="/tmp")
    actual = ActualEffects(creates=["/tmp/a"], cwd_after="/tmp")
    assert effects_match_prediction(predicted, actual) is True
    assert effects_match_prediction("bad", actual) is False


def test_revalidate_reports_stale_and_refreshes_snapshot(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "tracked.txt"
    target.write_text("v1\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(TouchCommand(path="tracked.txt", no_create=True), snapshot)
    record = plan_store.get(result.plan_id)
    target.write_text("v2\n", encoding="utf-8")

    stale_report, _ = revalidate_plan(record, snapshot, refresh_on_drift=False)
    assert stale_report.status == "stale"
    refreshed_report, refreshed = revalidate_plan(record, snapshot)
    assert refreshed_report.status == "stale"
    assert refreshed_report.refreshed_paths


def test_refresh_snapshot_paths_edge_cases(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    missing = str((workspace / "missing").resolve())
    updated, refreshed = refresh_snapshot_paths(snapshot, {missing})
    assert refreshed == [missing]
    assert missing not in updated.nodes

    ignored = workspace / ".venv"
    ignored.mkdir()
    updated, refreshed = refresh_snapshot_paths(snapshot, {str(ignored.resolve())})
    assert refreshed == []

    rootless = WorkspaceSnapshot(
        snapshot_id="snap_test",
        session=snapshot.session,
        generated_at_ns=1,
        nodes={},
    )
    updated, _ = refresh_snapshot_paths(rootless, {str(workspace.resolve())})
    assert str(workspace.resolve()) in updated.nodes


def test_fingerprint_node_and_persistence_restore_errors() -> None:
    node = SnapshotNode(path="/tmp/a", parent=None, kind="file", size=1, mode=0o644, mtime_ns=1)
    assert fingerprint_node(node).startswith("file:")
    with pytest.raises(ValueError, match="missing command_model_name"):
        _restore_command(None, {})
    with pytest.raises(ValueError, match="unknown persisted command model"):
        _restore_command("MissingCommand", {})


def test_runtime_persistence_round_trip(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("VSH_PERSIST", "1")
    store_root = tmp_path / "persist"
    monkeypatch.setenv("VSH_DATA_DIR", str(store_root))
    monkeypatch.setattr(persistence_store, "root", store_root)
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    store = PersistenceStore(root=store_root)
    assert store.load_snapshot(snapshot.snapshot_id).snapshot_id == snapshot.snapshot_id


def test_execute_approved_error_paths(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(MkdirCommand(path="pkg"), snapshot)
    token = approve_plan(result.plan_id)
    execute_approved(token.token)
    with pytest.raises(ValueError, match="plan already executed"):
        execute_approved(token.token)


def test_realfs_helpers_reject_invalid_snapshot(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    unchanged = _apply_session_updates(snapshot, ActualEffects())
    assert unchanged is snapshot
    _run_extension_hooks(snapshot, ActualEffects())


def test_dispatch_rejects_read_command_without_path(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))
    command = CatCommand.model_construct(path="a.txt")
    object.__setattr__(command, "path", None)
    with pytest.raises(ValueError, match="unsupported read command"):
        apply_command(command, ctx)


class _Runner:
    def verify(self, snapshot: WorkspaceSnapshot, touched_paths: list[str]) -> list[str]:
        return touched_paths


def test_execute_approved_stale_with_persistence_enabled(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store_root = tmp_path / "persist"
    monkeypatch.setenv("VSH_PERSIST", "1")
    monkeypatch.setenv("VSH_DATA_DIR", str(store_root))
    monkeypatch.setattr(persistence_store, "root", store_root)
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "tracked.txt"
    target.write_text("v1\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(TouchCommand(path="tracked.txt", no_create=True), snapshot)
    token = approve_plan(result.plan_id)
    target.write_text("v2\n", encoding="utf-8")
    execution = execute_approved(token.token)
    assert execution.applied is False
    assert execution.revalidation.status == "stale"


def test_execute_approved_persists_successful_plan(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    store_root = tmp_path / "persist"
    monkeypatch.setenv("VSH_PERSIST", "1")
    monkeypatch.setenv("VSH_DATA_DIR", str(store_root))
    monkeypatch.setattr(persistence_store, "root", store_root)
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(MkdirCommand(path="pkg"), snapshot)
    token = approve_plan(result.plan_id)
    execution = execute_approved(token.token)
    assert execution.applied is True
    assert (store_root / "plans" / f"{result.plan_id}.json").exists()


def test_fingerprint_path_detects_symlink(tmp_path: Path) -> None:
    target = tmp_path / "file.txt"
    target.write_text("x\n", encoding="utf-8")
    link = tmp_path / "link.txt"
    link.symlink_to(target)
    from vsh.snapshot.fingerprint import fingerprint_path

    assert fingerprint_path(str(link)).startswith("symlink:")


def test_refresh_directory_children_skips_missing_root_node(tmp_path: Path) -> None:
    from vsh.snapshot.refresh import _refresh_directory_children

    workspace = tmp_path / "workspace"
    workspace.mkdir()
    nodes: dict[str, SnapshotNode] = {}
    _refresh_directory_children(nodes, workspace)
    assert nodes == {}


def test_realfs_runs_shadow_workspace_runner(tmp_path: Path) -> None:
    extensions.shadow_workspace_runners.append(_Runner())
    try:
        workspace = tmp_path / "workspace"
        workspace.mkdir()
        snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
        result = simulate_command(MkdirCommand(path="pkg"), snapshot)
        token = approve_plan(result.plan_id)
        execution = execute_approved(token.token)
        assert execution.applied is True
    finally:
        extensions.shadow_workspace_runners.clear()


def test_execute_approved_returns_apply_failure(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(MkdirCommand(path="pkg/nested", parents=False), snapshot)
    token = approve_plan(result.plan_id)

    def fail_apply(*_args: object, **_kwargs: object) -> ActualEffects:
        raise ValueError("boom")

    monkeypatch.setattr("vsh.execute.realfs.apply_command", fail_apply)
    execution = execute_approved(token.token)
    assert execution.applied is False
    assert execution.reason == "boom"
