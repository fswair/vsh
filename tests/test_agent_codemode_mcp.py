from __future__ import annotations as _annotations

import tempfile
from pathlib import Path
from typing import Any, cast

import pytest
from pydantic_ai.mcp import CallToolFunc
from pydantic_ai.models.test import TestModel
from pydantic_ai.tools import RunContext

from vsh.agent import VshAgentDeps, VshCapability, create_vsh_agent
from vsh.agent._codemode_mcp import CODEMODE_MCP_TOOL_NAMES, inject_workspace_mcp_call


def _run_context(deps: VshAgentDeps) -> RunContext[VshAgentDeps]:
    return RunContext(
        deps=deps,
        model=cast(Any, None),
        usage=cast(Any, None),
        prompt=None,
        run_step=0,
    )


def _call_tool(func: Any) -> CallToolFunc:
    return cast(CallToolFunc, func)


def test_vsh_capability_codemode_mcp_registers_compact_tools() -> None:
    test_model = TestModel(call_tools=[])
    capability = VshCapability("/tmp/workspace", codemode_mcp=True)
    toolset = capability.get_toolset()
    assert toolset is not None
    agent, cap = create_vsh_agent(test_model, "/tmp/workspace", vsh=capability)
    agent.run_sync("list tools", deps=cap.deps)
    params = test_model.last_model_request_parameters
    assert params is not None
    registered = {tool.name for tool in params.function_tools}
    assert registered == set(CODEMODE_MCP_TOOL_NAMES)


def test_vsh_capability_legacy_toolset_when_codemode_disabled() -> None:
    capability = VshCapability("/tmp/workspace", codemode_mcp=False)
    toolset = capability.get_toolset()
    assert toolset is not None
    toolset_with_tools = cast(Any, toolset)
    assert "vsh_search" in toolset_with_tools.tools


@pytest.mark.anyio
async def test_inject_workspace_mcp_call_sets_snapshot_id() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        deps = VshAgentDeps.from_path(tmp)
        ctx = _run_context(deps)

        async def fake_call_tool(name: str, args: dict[str, object]) -> dict[str, str]:
            assert args["workspace_root"] == deps.workspace_root
            return {"snapshot_id": "snap_test"}

        result = await inject_workspace_mcp_call(
            ctx,
            _call_tool(fake_call_tool),
            "snapshot_workspace",
            {},
        )

        assert result["snapshot_id"] == "snap_test"
        assert deps.snapshot_id == "snap_test"


@pytest.mark.anyio
async def test_inject_workspace_mcp_call_sets_plan_id() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        deps = VshAgentDeps.from_path(tmp)
        deps.snapshot_id = "snap_existing"
        ctx = _run_context(deps)

        async def fake_call_tool(name: str, args: dict[str, object]) -> dict[str, str]:
            assert name == "simulate"
            assert args["snapshot_id"] == "snap_existing"
            return {"plan_id": "plan_test"}

        result = await inject_workspace_mcp_call(
            ctx,
            _call_tool(fake_call_tool),
            "simulate",
            {"tool_name": "vsh_list", "params": {"path": "."}},
        )

        assert result["plan_id"] == "plan_test"
        assert deps.last_plan_id == "plan_test"


@pytest.mark.anyio
async def test_inject_workspace_mcp_call_ignores_non_string_ids() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        deps = VshAgentDeps.from_path(tmp)
        ctx = _run_context(deps)

        async def fake_call_tool(name: str, args: dict[str, object]) -> dict[str, object]:
            if name == "snapshot_workspace":
                return {"snapshot_id": 123}
            return {"plan_id": 456}

        await inject_workspace_mcp_call(
            ctx,
            _call_tool(fake_call_tool),
            "snapshot_workspace",
            {},
        )
        assert deps.snapshot_id is None

        deps.snapshot_id = "snap"
        await inject_workspace_mcp_call(
            ctx,
            _call_tool(fake_call_tool),
            "simulate",
            {"tool_name": "vsh_list", "params": {"path": "."}},
        )
        assert deps.last_plan_id is None


def test_vsh_capability_after_tool_execute_tracks_codemode_names(tmp_path: Path) -> None:
    import asyncio

    from pydantic_ai.messages import ToolCallPart
    from pydantic_ai.tools import ToolDefinition

    capability = VshCapability(tmp_path, codemode_mcp=True)
    deps = capability.deps
    ctx = _run_context(deps)

    async def run_snapshot() -> Any:
        return await capability.after_tool_execute(
            ctx,
            call=ToolCallPart(tool_name="snapshot_workspace", args={}, tool_call_id="snap-1"),
            tool_def=ToolDefinition(
                name="snapshot_workspace",
                description="",
                parameters_json_schema={},
            ),
            args={},
            result={"snapshot_id": "snap_codemode"},
        )

    asyncio.run(run_snapshot())
    assert deps.snapshot_id == "snap_codemode"

    async def run_simulate() -> Any:
        return await capability.after_tool_execute(
            ctx,
            call=ToolCallPart(tool_name="simulate", args={}, tool_call_id="sim-1"),
            tool_def=ToolDefinition(
                name="simulate",
                description="",
                parameters_json_schema={},
            ),
            args={},
            result={"plan_id": "plan_codemode", "decision": "approve"},
        )

    asyncio.run(run_simulate())
    assert deps.last_plan_id == "plan_codemode"


def test_vsh_capability_after_tool_execute_ignores_invalid_snapshot_id(tmp_path: Path) -> None:
    import asyncio

    from pydantic_ai.messages import ToolCallPart
    from pydantic_ai.tools import ToolDefinition

    capability = VshCapability(tmp_path, codemode_mcp=True)
    deps = capability.deps
    ctx = _run_context(deps)

    async def run_snapshot() -> Any:
        return await capability.after_tool_execute(
            ctx,
            call=ToolCallPart(tool_name="snapshot_workspace", args={}, tool_call_id="snap-2"),
            tool_def=ToolDefinition(
                name="snapshot_workspace",
                description="",
                parameters_json_schema={},
            ),
            args={},
            result={"snapshot_id": 999},
        )

    asyncio.run(run_snapshot())
    assert deps.snapshot_id is None


def test_vsh_capability_after_tool_execute_ignores_non_dict_snapshot_result(
    tmp_path: Path,
) -> None:
    import asyncio

    from pydantic_ai.messages import ToolCallPart
    from pydantic_ai.tools import ToolDefinition

    capability = VshCapability(tmp_path, codemode_mcp=True)
    deps = capability.deps
    ctx = _run_context(deps)

    async def run_snapshot() -> Any:
        return await capability.after_tool_execute(
            ctx,
            call=ToolCallPart(tool_name="snapshot_workspace", args={}, tool_call_id="snap-3"),
            tool_def=ToolDefinition(
                name="snapshot_workspace",
                description="",
                parameters_json_schema={},
            ),
            args={},
            result="not-a-dict",
        )

    asyncio.run(run_snapshot())
    assert deps.snapshot_id is None


@pytest.mark.anyio
async def test_inject_workspace_mcp_call_simulate_without_snapshot_skips_patch() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        deps = VshAgentDeps.from_path(tmp)
        ctx = _run_context(deps)

        async def fake_call_tool(name: str, args: dict[str, object]) -> str:
            assert "snapshot_id" not in args
            return "ok"

        result = await inject_workspace_mcp_call(
            ctx,
            _call_tool(fake_call_tool),
            "simulate",
            {"tool_name": "vsh_list", "params": {"path": "."}},
        )

        assert result == "ok"


@pytest.mark.anyio
async def test_inject_workspace_mcp_call_tracks_apply_batch_plan_id() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        deps = VshAgentDeps.from_path(tmp)
        deps.snapshot_id = "snap_existing"
        ctx = _run_context(deps)

        async def fake_call_tool(name: str, args: dict[str, object]) -> dict[str, object]:
            assert args["workspace_root"] == deps.workspace_root
            assert args["cwd"] == "."
            assert args["snapshot_id"] == "snap_existing"
            return {
                "snapshot_id": "snap_next",
                "steps": [{"plan_id": "plan_final"}, "skip-me", {"plan_id": 123}],
            }

        result = await inject_workspace_mcp_call(
            ctx,
            _call_tool(fake_call_tool),
            "apply_batch",
            {"steps": []},
        )

        assert result["snapshot_id"] == "snap_next"
        assert deps.snapshot_id == "snap_next"
        assert deps.last_plan_id == "plan_final"


@pytest.mark.anyio
async def test_inject_workspace_mcp_call_apply_batch_without_string_ids() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        deps = VshAgentDeps.from_path(tmp)
        ctx = _run_context(deps)

        async def fake_call_tool(name: str, args: dict[str, object]) -> dict[str, object]:
            return {"snapshot_id": 123, "steps": [{"plan_id": 456}]}

        await inject_workspace_mcp_call(
            ctx,
            _call_tool(fake_call_tool),
            "apply_batch",
            {"steps": []},
        )

        assert deps.snapshot_id is None
        assert deps.last_plan_id is None


@pytest.mark.anyio
async def test_inject_workspace_mcp_call_apply_batch_ignores_non_list_steps() -> None:
    with tempfile.TemporaryDirectory() as tmp:
        deps = VshAgentDeps.from_path(tmp)
        ctx = _run_context(deps)

        async def fake_call_tool(name: str, args: dict[str, object]) -> dict[str, object]:
            return {"steps": "not-a-list"}

        await inject_workspace_mcp_call(
            ctx,
            _call_tool(fake_call_tool),
            "apply_batch",
            {"steps": []},
        )

        assert deps.last_plan_id is None


def test_vsh_capability_after_tool_execute_ignores_invalid_plan_id(tmp_path: Path) -> None:
    import asyncio

    from pydantic_ai.messages import ToolCallPart
    from pydantic_ai.tools import ToolDefinition

    capability = VshCapability(tmp_path, codemode_mcp=True)
    deps = capability.deps
    ctx = _run_context(deps)

    async def run_simulate() -> Any:
        return await capability.after_tool_execute(
            ctx,
            call=ToolCallPart(tool_name="simulate", args={}, tool_call_id="sim-2"),
            tool_def=ToolDefinition(
                name="simulate",
                description="",
                parameters_json_schema={},
            ),
            args={},
            result={"plan_id": 123},
        )

    asyncio.run(run_simulate())
    assert deps.last_plan_id is None
