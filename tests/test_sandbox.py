from __future__ import annotations as _annotations

from pathlib import Path

import pytest

from vsh.plans.models import SimulationResult
from vsh.sandbox import (
    advance_snapshot,
    allowed_effect_kinds,
    mount_mode_for_policy,
    policy_allows_simulation,
    run_vsh_sandbox,
)
from vsh.sandbox.policy import classify_simulation_effects, effect_kinds_allowed_by_policy
from vsh.schemas import (
    CatCommand,
    CdCommand,
    MkdirCommand,
    MoveCommand,
    RemoveCommand,
    TouchCommand,
)
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_classify_simulation_effects_marks_reads_and_writes() -> None:
    kinds = classify_simulation_effects(
        reads=["/tmp/a"],
        creates=["/tmp/b"],
        updates=[],
        deletes=[],
        renames=[],
        content_reads={"/tmp/a"},
    )
    assert kinds == {"read", "write"}


def test_allowed_effect_kinds_alias() -> None:
    assert allowed_effect_kinds("read_only") == effect_kinds_allowed_by_policy("read_only")


def test_classify_effect_variants() -> None:
    assert classify_simulation_effects(
        reads=[],
        creates=[],
        updates=[],
        deletes=["/tmp/x"],
        renames=[],
    ) == {"delete"}
    assert classify_simulation_effects(
        reads=[],
        creates=[],
        updates=[],
        deletes=[],
        renames=[("/a", "/b")],
    ) == {"rename"}
    assert classify_simulation_effects(
        reads=[],
        creates=[],
        updates=[],
        deletes=[],
        renames=[],
        content_reads={"/tmp/x"},
    ) == {"read"}
    assert policy_allows_simulation("yolo", {"read", "write", "delete", "rename"})


def test_advance_snapshot_updates_existing_node_revision(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "existing.txt"
    target.write_text("x\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    resolved = str(target.resolve())
    before = snapshot.nodes[resolved].revision
    touch = simulate_command(TouchCommand(path="existing.txt", no_create=True), snapshot)
    advanced = advance_snapshot(snapshot, touch)
    assert advanced.nodes[resolved].revision == before + 1


def test_mcp_vsh_sandbox_tool(tmp_path: Path) -> None:
    from vsh.mcp import tools as mcp_tools

    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    payload = mcp_tools.vsh_sandbox(
        'return simulate("vsh_list", {"path": "."})["decision"]',
        snapshot.snapshot_id,
        policy="read_only",
    )
    assert payload["error"] is None
    assert payload["calls"]
    assert effect_kinds_allowed_by_policy("yolo") is None
    assert policy_allows_simulation("read_only", {"read"})
    assert not policy_allows_simulation("read_only", {"write"})
    assert policy_allows_simulation("no_delete", {"read", "write", "rename"})
    assert not policy_allows_simulation("no_delete", {"delete"})
    assert mount_mode_for_policy("read_only") == "read-only"
    assert mount_mode_for_policy("read_write") == "overlay"


def test_advance_snapshot_applies_create_and_cwd(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    touch = simulate_command(TouchCommand(path="new.txt"), snapshot)
    advanced = advance_snapshot(snapshot, touch)
    target = str((workspace / "new.txt").resolve())
    assert target in advanced.nodes


def test_run_vsh_sandbox_chains_simulations(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    code = """
simulate("vsh_touch", {"path": "x.txt"})
simulate("vsh_mkdir", {"path": "foo", "parents": True})
return "done"
"""
    result = run_vsh_sandbox(code, snapshot.snapshot_id, policy="read_write")
    assert result.error is None
    assert result.output == "done"
    assert len(result.calls) == 2
    assert result.calls[0].tool_name == "vsh_touch"
    assert result.calls[1].tool_name == "vsh_mkdir"


def test_run_vsh_sandbox_read_only_blocks_touch(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    code = 'simulate("vsh_touch", {"path": "x.txt"})'
    result = run_vsh_sandbox(code, snapshot.snapshot_id, policy="read_only")
    assert result.error is not None
    assert "read_only" in result.error


def test_run_vsh_sandbox_exposes_discovery_helpers(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    code = """
names = [item["name"] for item in search("touch")]
schema = get_schema("vsh_touch")
return names[0]
"""
    result = run_vsh_sandbox(code, snapshot.snapshot_id, policy="read_only")
    assert result.error is None
    assert result.output == "vsh_touch"


def test_run_vsh_sandbox_returns_runtime_errors(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = run_vsh_sandbox("1/0", snapshot.snapshot_id)
    assert result.error is not None


def test_vsh_sandbox_session_simulate_raises_on_reject(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    code = 'simulate("vsh_cd", {"path": "/etc"})'
    result = run_vsh_sandbox(code, snapshot.snapshot_id, policy="read_only")
    assert result.error is not None


def test_advance_snapshot_handles_rename(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    source = workspace / "src.txt"
    source.write_text("x\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    move = simulate_command(MoveCommand(src="src.txt", dst="dst.txt", overwrite=True), snapshot)
    advanced = advance_snapshot(snapshot, move)
    dst = str((workspace / "dst.txt").resolve())
    assert dst in advanced.nodes


def test_run_vsh_sandbox_returns_syntax_errors(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = run_vsh_sandbox("def broken(", snapshot.snapshot_id)
    assert result.error is not None


def test_run_vsh_sandbox_returns_unexpected_runtime_errors(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    def boom(*_args: object, **_kwargs: object) -> None:
        raise RuntimeError("boom")

    monkeypatch.setattr("vsh.sandbox.runner.Monty.run", boom)
    result = run_vsh_sandbox("return 1", snapshot.snapshot_id)
    assert result.error == "boom"


def test_mount_modes_for_additional_policies() -> None:
    assert mount_mode_for_policy("delete_only") == "read-only"
    assert mount_mode_for_policy("write_only") == "overlay"
    assert mount_mode_for_policy("no_read") == "read-only"


def test_advance_snapshot_removes_deleted_nodes(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "drop.txt"
    target.write_text("x\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    remove = simulate_command(RemoveCommand(path="drop.txt"), snapshot)
    advanced = advance_snapshot(snapshot, remove)
    assert str(target.resolve()) not in advanced.nodes


def test_advance_snapshot_creates_missing_rename_target(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    fake_result = SimulationResult(
        plan_id="plan_test",
        command=CatCommand(path="x"),
        shell_preview="cat x",
        decision="approve",
        execution_eligible=True,
        predicted_effects=simulate_command(
            CatCommand(path="x"), snapshot
        ).predicted_effects.model_copy(
            update={"renames": [(str(workspace / "missing.txt"), str(workspace / "new.txt"))]}
        ),
        journal=simulate_command(CatCommand(path="x"), snapshot).journal,
    )
    advanced = advance_snapshot(snapshot, fake_result)
    assert str((workspace / "new.txt").resolve()) in advanced.nodes


def test_advance_snapshot_updates_cwd_and_missing_update_path(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    sub = workspace / "sub"
    sub.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    cd = simulate_command(CdCommand(path="sub"), snapshot)
    advanced = advance_snapshot(snapshot, cd)
    assert advanced.session.cwd_logical == str(sub.resolve())

    ghost = str((workspace / "ghost.txt").resolve())
    touch = simulate_command(TouchCommand(path="ghost.txt", no_create=True), snapshot)
    effects = touch.predicted_effects.model_copy(update={"creates": [], "updates": [ghost]})
    fake = touch.model_copy(update={"predicted_effects": effects})
    advanced = advance_snapshot(snapshot, fake)
    assert ghost in advanced.nodes


def test_advance_snapshot_creates_nested_paths(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    mkdir = simulate_command(MkdirCommand(path="nested/deep", parents=True), snapshot)
    advanced = advance_snapshot(snapshot, mkdir)
    target = str((workspace / "nested" / "deep").resolve())
    assert target in advanced.nodes


def test_advance_snapshot_remove_node_without_parent(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    orphan = str((workspace / "solo.txt").resolve())
    snapshot = snapshot.model_copy(
        update={
            "nodes": {
                **snapshot.nodes,
                orphan: snapshot.nodes[str(workspace.resolve())].model_copy(
                    update={"path": orphan, "parent": None, "children": []}
                ),
            }
        }
    )
    remove = simulate_command(RemoveCommand(path="solo.txt"), snapshot)
    advanced = advance_snapshot(snapshot, remove)
    assert orphan not in advanced.nodes


def test_advance_snapshot_rename_without_parent_in_nodes(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    source = workspace / "solo.txt"
    source.write_text("x\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    src = str(source.resolve())
    dst = str((workspace / "moved.txt").resolve())
    snapshot = snapshot.model_copy(
        update={
            "nodes": {
                **snapshot.nodes,
                src: snapshot.nodes[str(workspace.resolve())].model_copy(
                    update={"path": src, "parent": "/missing-parent", "children": []}
                ),
            }
        }
    )
    move = simulate_command(MoveCommand(src="solo.txt", dst="moved.txt", overwrite=True), snapshot)
    advanced = advance_snapshot(snapshot, move)
    assert dst in advanced.nodes


def test_advance_snapshot_skips_cwd_update_when_unset(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    touch = simulate_command(TouchCommand(path="plain.txt"), snapshot)
    effects = touch.predicted_effects.model_copy(update={"cwd_after": None})
    fake = touch.model_copy(update={"predicted_effects": effects})
    advanced = advance_snapshot(snapshot, fake)
    assert advanced.session.cwd_logical == snapshot.session.cwd_logical


def test_advance_snapshot_ensure_node_skips_parent_link_for_root_path(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    touch = simulate_command(TouchCommand(path="x.txt"), snapshot)
    fake = touch.model_copy(
        update={"predicted_effects": touch.predicted_effects.model_copy(update={"creates": ["/"]})}
    )
    advanced = advance_snapshot(snapshot, fake)
    assert "/" in advanced.nodes


def test_advance_snapshot_ensure_node_skips_existing_child_link(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    root = str(workspace.resolve())
    child = str((workspace / "linked.txt").resolve())
    root_node = snapshot.nodes[root]
    snapshot = snapshot.model_copy(
        update={
            "nodes": {**snapshot.nodes, root: root_node.model_copy(update={"children": [child]})}
        }
    )
    touch = simulate_command(TouchCommand(path="linked.txt"), snapshot)
    advanced = advance_snapshot(snapshot, touch)
    assert child in advanced.nodes


def test_advance_snapshot_ensure_node_is_idempotent(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    touch = simulate_command(TouchCommand(path="dup.txt"), snapshot)
    advanced = advance_snapshot(snapshot, touch)
    again = advance_snapshot(advanced, touch)
    assert str((workspace / "dup.txt").resolve()) in again.nodes


def test_advance_snapshot_remove_handles_missing_parent(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    orphan = str((workspace / "orphan.txt").resolve())
    snapshot = snapshot.model_copy(
        update={
            "nodes": {
                **snapshot.nodes,
                orphan: snapshot.nodes[str(workspace.resolve())].model_copy(
                    update={"path": orphan, "parent": "/missing", "children": []}
                ),
            }
        }
    )
    remove = simulate_command(RemoveCommand(path="orphan.txt"), snapshot)
    advanced = advance_snapshot(snapshot, remove)
    assert orphan not in advanced.nodes


def test_advance_snapshot_rename_updates_parent_children(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    source = workspace / "src.txt"
    source.write_text("x\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    root = str(workspace.resolve())
    src = str(source.resolve())
    dst = str((workspace / "dst.txt").resolve())
    root_node = snapshot.nodes[root]
    snapshot = snapshot.model_copy(
        update={"nodes": {**snapshot.nodes, root: root_node.model_copy(update={"children": [src]})}}
    )
    move = simulate_command(MoveCommand(src="src.txt", dst="dst.txt", overwrite=True), snapshot)
    advanced = advance_snapshot(snapshot, move)
    assert dst in advanced.nodes[root].children
