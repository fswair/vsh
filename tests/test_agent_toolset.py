from __future__ import annotations as _annotations

import tempfile
from pathlib import Path

import pytest
from pydantic_ai import Agent
from pydantic_ai.messages import ModelResponse, TextPart, ToolCallPart
from pydantic_ai.models.function import FunctionModel
from pydantic_ai.models.test import TestModel

from vsh.agent import VshAgentDeps, create_vsh_function_toolset


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


def test_agent_module_unknown_attribute_raises() -> None:
    import vsh.agent as agent_module

    with pytest.raises(AttributeError, match="has no attribute 'missing_tool'"):
        getattr(agent_module, "missing_" + "tool")


def test_create_vsh_function_toolset_registers_tools() -> None:
    toolset = create_vsh_function_toolset()

    assert set(toolset.tools) == {
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


def test_vsh_search_and_schema_tools_run_through_agent() -> None:
    step = 0

    def next_step(_messages: object, _info: object) -> ModelResponse:
        nonlocal step
        step += 1
        if step == 1:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_search",
                        args={"query": "list"},
                        tool_call_id="search",
                    )
                ]
            )
        if step == 2:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_get_schema",
                        args={"name": "vsh_list"},
                        tool_call_id="schema",
                    )
                ]
            )
        return ModelResponse(parts=[TextPart(content="done")])

    toolset = create_vsh_function_toolset()
    agent = Agent(FunctionModel(next_step), deps_type=VshAgentDeps, toolsets=[toolset])
    result = agent.run_sync("discover commands", deps=VshAgentDeps(workspace_root="/tmp"))

    assert result.output == "done"


def test_vsh_toolset_updates_deps_during_agent_run() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        workspace = Path(tmp)
        (workspace / "README.md").write_text("hello\n", encoding="utf-8")
        deps = VshAgentDeps.from_path(workspace)
        toolset = create_vsh_function_toolset()
        agent = Agent(
            _scripted_snapshot_and_simulate(),
            deps_type=VshAgentDeps,
            toolsets=[toolset],
        )

        agent.run_sync("Inspect the workspace.", deps=deps)

        assert deps.snapshot_id is not None
        assert deps.last_plan_id is not None


def test_vsh_simulate_requires_snapshot() -> None:
    toolset = create_vsh_function_toolset()
    test_model = TestModel(call_tools=["vsh_simulate"])
    agent = Agent(test_model, deps_type=VshAgentDeps, toolsets=[toolset])
    deps = VshAgentDeps(workspace_root="/tmp")

    with pytest.raises(Exception, match="snapshot_id is missing"):
        agent.run_sync("simulate now", deps=deps)


def test_vsh_approve_and_execute_helpers(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "README.md").write_text("hello\n", encoding="utf-8")
    deps = VshAgentDeps.from_path(workspace)
    toolset = create_vsh_function_toolset()

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
                        args={
                            "tool_name": "vsh_list",
                            "params": {
                                "path": ".",
                                "all": True,
                                "long": True,
                                "raw_command": "ls -la .",
                            },
                        },
                        tool_call_id="sim",
                    )
                ]
            )
        if step == 3:
            return ModelResponse(
                parts=[ToolCallPart(tool_name="vsh_approve", args={}, tool_call_id="approve")]
            )
        if step == 4:
            return ModelResponse(
                parts=[
                    ToolCallPart(tool_name="vsh_execute_approved", args={}, tool_call_id="execute")
                ]
            )
        return ModelResponse(parts=[TextPart(content="done")])

    agent = Agent(FunctionModel(next_step), deps_type=VshAgentDeps, toolsets=[toolset])
    agent.run_sync("approve and execute", deps=deps)

    assert deps.last_approval_token is not None


def test_vsh_approve_requires_plan_id() -> None:
    toolset = create_vsh_function_toolset()
    test_model = TestModel(call_tools=["vsh_approve"])
    agent = Agent(test_model, deps_type=VshAgentDeps, toolsets=[toolset])

    with pytest.raises(Exception, match="plan_id is missing"):
        agent.run_sync("approve", deps=VshAgentDeps(workspace_root="/tmp"))


def test_vsh_sandbox_requires_snapshot() -> None:
    toolset = create_vsh_function_toolset()
    test_model = TestModel(call_tools=["vsh_sandbox"])
    agent = Agent(test_model, deps_type=VshAgentDeps, toolsets=[toolset])

    with pytest.raises(Exception, match="snapshot_id is missing"):
        agent.run_sync(
            "sandbox now",
            deps=VshAgentDeps(workspace_root="/tmp"),
        )


def test_vsh_sandbox_runs_through_agent(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    deps = VshAgentDeps.from_path(workspace)
    toolset = create_vsh_function_toolset()
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
                        tool_name="vsh_sandbox",
                        args={
                            "code": 'return simulate("vsh_list", {"path": "."})["decision"]',
                            "policy": "read_only",
                        },
                        tool_call_id="sandbox",
                    )
                ]
            )
        return ModelResponse(parts=[TextPart(content="done")])

    agent = Agent(FunctionModel(next_step), deps_type=VshAgentDeps, toolsets=[toolset])
    result = agent.run_sync("batch simulate", deps=deps)

    assert result.output == "done"


def test_vsh_artifact_tools_run_through_agent(tmp_path: Path) -> None:
    from vsh.artifacts import MemoryArtifactStore

    store = MemoryArtifactStore()
    record = store.put(
        tool_name="vsh_simulate",
        payload=b"hello-world",
        content_type="text/plain; charset=utf-8",
    )
    step = 0

    def next_step(_messages: object, _info: object) -> ModelResponse:
        nonlocal step
        step += 1
        if step == 1:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_get_artifact",
                        args={"artifact_id": record.ref.artifact_id, "offset": 0, "limit": 5},
                        tool_call_id="get",
                    )
                ]
            )
        if step == 2:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_index_artifact",
                        args={
                            "artifact_id": record.ref.artifact_id,
                            "title": "hello",
                            "tags": ["demo"],
                        },
                        tool_call_id="index",
                    )
                ]
            )
        if step == 3:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_search_artifacts",
                        args={"query": "demo"},
                        tool_call_id="search",
                    )
                ]
            )
        return ModelResponse(parts=[TextPart(content="done")])

    deps = VshAgentDeps(workspace_root=str(tmp_path), artifact_store=store)
    toolset = create_vsh_function_toolset()
    agent = Agent(FunctionModel(next_step), deps_type=VshAgentDeps, toolsets=[toolset])
    result = agent.run_sync("artifact flow", deps=deps)
    assert result.output == "done"


def test_vsh_simulate_execution_reason_kwarg_runs_through_agent(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
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
                        args={
                            "tool_name": "vsh_touch",
                            "params": {"path": "new.txt"},
                            "execution_reason": "create marker",
                        },
                        tool_call_id="sim",
                    )
                ]
            )
        return ModelResponse(parts=[TextPart(content="done")])

    deps = VshAgentDeps.from_path(workspace)
    toolset = create_vsh_function_toolset()
    agent = Agent(FunctionModel(next_step), deps_type=VshAgentDeps, toolsets=[toolset])
    agent.run_sync("simulate touch", deps=deps)
    assert deps.last_plan_id is not None


def test_vsh_execute_requires_approval_token() -> None:
    toolset = create_vsh_function_toolset()
    test_model = TestModel(call_tools=["vsh_execute_approved"])
    agent = Agent(test_model, deps_type=VshAgentDeps, toolsets=[toolset])

    with pytest.raises(Exception, match="approval_token is missing"):
        agent.run_sync("execute", deps=VshAgentDeps(workspace_root="/tmp"))
