from __future__ import annotations as _annotations

import tempfile
from pathlib import Path
from typing import Any, cast

import pytest
from pydantic_ai.messages import ModelResponse, TextPart, ToolCallPart
from pydantic_ai.models.function import FunctionModel
from pydantic_ai.models.test import TestModel

from vsh.agent import VshAgentDeps, VshCapability, create_vsh_agent


def _scripted_snapshot_and_simulate() -> FunctionModel:
    step = 0

    def next_step(_messages: object, _info: object) -> ModelResponse:
        nonlocal step
        step += 1
        if step == 1:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_snapshot_workspace",
                        args={},
                        tool_call_id="snap",
                    )
                ]
            )
        if step == 2:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_simulate",
                        args={"tool_name": "vsh_list", "params": {"path": "."}},
                        tool_call_id="sim",
                    )
                ]
            )
        return ModelResponse(parts=[TextPart(content="done")])

    return FunctionModel(next_step)


def test_vsh_capability_registers_tools() -> None:
    capability = VshCapability("/tmp/workspace", codemode_mcp=False)
    toolset = capability.get_toolset()
    assert toolset is not None
    toolset_with_tools = cast(Any, toolset)
    assert set(toolset_with_tools.tools) == {
        "vsh_search",
        "vsh_get_schema",
        "vsh_snapshot_workspace",
        "vsh_simulate",
        "vsh_approve",
        "vsh_execute_approved",
        "vsh_sandbox",
        "vsh_get_artifact",
        "vsh_index_artifact",
        "vsh_search_artifacts",
    }


def test_vsh_capability_accepts_artifact_store_and_spill_bytes(tmp_path: Path) -> None:
    from vsh.artifacts import MemoryArtifactStore

    store = MemoryArtifactStore()
    capability = VshCapability(tmp_path, artifact_store=store, artifact_spill_bytes=2048)
    assert capability.deps.artifact_store is store
    assert capability.deps.artifact_spill_bytes == 2048


def test_vsh_capability_exposes_workspace_deps() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        capability = VshCapability(tmp)
        assert capability.workspace_root == str(Path(tmp).resolve())
        assert isinstance(capability.deps, VshAgentDeps)
        assert capability.deps.workspace_root == capability.workspace_root
        assert capability.deps is not capability


def test_create_vsh_agent_wires_capability_tools() -> None:
    test_model = TestModel(call_tools=[])
    agent, capability = create_vsh_agent(test_model, "/tmp/workspace", codemode_mcp=False)

    agent.run_sync("list tools", deps=capability.deps)
    params = test_model.last_model_request_parameters
    assert params is not None
    assert {tool.name for tool in params.function_tools} == {
        "vsh_search",
        "vsh_get_schema",
        "vsh_snapshot_workspace",
        "vsh_simulate",
        "vsh_approve",
        "vsh_execute_approved",
        "vsh_sandbox",
        "vsh_get_artifact",
        "vsh_index_artifact",
        "vsh_search_artifacts",
    }


def test_vsh_capability_updates_deps_during_agent_run() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        workspace = Path(tmp)
        (workspace / "README.md").write_text("hello\n", encoding="utf-8")
        agent, capability = create_vsh_agent(
            _scripted_snapshot_and_simulate(),
            workspace,
            codemode_mcp=False,
        )

        agent.run_sync("Inspect the workspace.", deps=capability.deps)

        assert capability.deps.snapshot_id is not None
        assert capability.deps.last_plan_id is not None


def test_vsh_capability_simulate_requires_snapshot() -> None:
    test_model = TestModel(call_tools=["vsh_simulate"])
    agent, capability = create_vsh_agent(test_model, "/tmp/workspace", codemode_mcp=False)

    with pytest.raises(Exception, match="snapshot_id is missing"):
        agent.run_sync("simulate now", deps=capability.deps)


def test_vsh_capability_supports_deferred_loading() -> None:
    capability = VshCapability("/tmp/workspace", defer_loading=True)
    assert capability.defer_loading is True
    assert capability.id == "vsh"


def test_vsh_capability_can_be_reused_across_agents() -> None:
    capability = VshCapability("/tmp/workspace")
    agent_a, _ = create_vsh_agent(TestModel(call_tools=[]), "/tmp/workspace", vsh=capability)
    agent_b, _ = create_vsh_agent(TestModel(call_tools=[]), "/tmp/workspace", vsh=capability)

    assert agent_a is not agent_b
    assert (
        capability.deps.workspace_root == "/private/tmp/workspace"
        or capability.deps.workspace_root.endswith("/tmp/workspace")
    )
