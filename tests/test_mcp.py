from __future__ import annotations as _annotations

from pathlib import Path

import pytest

from vsh.mcp import resources, server, tools
from vsh.plans.store import plan_store
from vsh.runtime import runtime
from vsh.schemas import PwdCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_mcp_server_registers_tools_and_resources() -> None:
    assert server.mcp.name == "vsh"


def test_workspace_snapshot_resource_builds_when_runtime_empty(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    monkeypatch.chdir(workspace)
    runtime.snapshots.clear()
    runtime.latest_snapshot_id = None

    payload = resources.workspace_snapshot_current()

    assert payload["session"]["workspace_root"] == str(workspace.resolve())


def test_workspace_projection_resource_builds_when_runtime_empty(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    monkeypatch.chdir(workspace)
    runtime.snapshots.clear()
    runtime.latest_snapshot_id = None

    payload = resources.workspace_projection_current()

    assert payload["workspace_root"] == str(workspace.resolve())


def test_simulation_record_resource_returns_plan_payload(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(PwdCommand(), snapshot)

    payload = resources.simulation_record(result.plan_id)

    assert payload["plan_id"] == result.plan_id
    assert payload["result"]["shell_preview"] == "pwd"


def test_mcp_tools_simulate_and_approve(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot_payload = tools.snapshot_workspace(str(workspace), cwd=str(workspace))

    simulation = tools.simulate("vsh_list", snapshot_payload["snapshot_id"], {"path": "."})
    approval = tools.approve(simulation["plan_id"])

    assert simulation["decision"] == "approve"
    assert approval["plan_id"] == simulation["plan_id"]
    assert plan_store.get(simulation["plan_id"]).approval_token is not None
