from __future__ import annotations as _annotations

from pathlib import Path
from typing import Any, cast

from pydantic_ai.messages import ModelRequest, ToolReturnPart
from pydantic_ai.models import ModelRequestContext
from pydantic_ai.tools import RunContext

from vsh.agent import VshAgentDeps, VshCapability, _artifact_spill
from vsh.artifacts import MemoryArtifactStore


def test_maybe_spill_tool_result_small_payload_unchanged() -> None:
    store = MemoryArtifactStore()
    payload = {"ok": True}
    assert (
        _artifact_spill.maybe_spill_tool_result(
            store,
            tool_name="vsh_simulate",
            result=payload,
            threshold=10_000,
        )
        is payload
    )


def test_maybe_spill_tool_result_spills_large_vsh_tool() -> None:
    store = MemoryArtifactStore()
    large = {"data": "x" * 10_000}
    spilled = _artifact_spill.maybe_spill_tool_result(
        store,
        tool_name="vsh_simulate",
        result=large,
        threshold=100,
    )
    assert isinstance(spilled, dict)
    assert "artifact_id" in spilled
    assert spilled["byte_size"] > 100


def test_maybe_spill_skips_non_vsh_and_artifact_tools() -> None:
    store = MemoryArtifactStore()
    payload = {"data": "x" * 10_000}
    assert (
        _artifact_spill.maybe_spill_tool_result(
            store,
            tool_name="other_tool",
            result=payload,
            threshold=10,
        )
        is payload
    )
    assert (
        _artifact_spill.maybe_spill_tool_result(
            store,
            tool_name="vsh_get_artifact",
            result=payload,
            threshold=10,
        )
        is payload
    )


def test_maybe_spill_passthrough_existing_artifact_ref() -> None:
    store = MemoryArtifactStore()
    ref = {
        "artifact_id": "abcd1234abcd1234",
        "content_hash": "hash",
        "byte_size": 1,
        "content_type": "application/json",
        "tool_name": "vsh_simulate",
        "preview": "p",
        "spilled_at_ns": 1,
    }
    assert (
        _artifact_spill.maybe_spill_tool_result(
            store,
            tool_name="vsh_simulate",
            result=ref,
            threshold=1,
        )
        == ref
    )


def test_sanitize_history_tool_returns_replaces_large_returns() -> None:
    store = MemoryArtifactStore()
    large = {"blob": "y" * 10_000}
    request = ModelRequest(
        parts=[
            ToolReturnPart(
                tool_name="vsh_simulate",
                content=large,
                tool_call_id="sim-1",
            )
        ]
    )
    sanitized = _artifact_spill.sanitize_history_tool_returns(
        [request],
        store,
        threshold=100,
    )
    part = sanitized[0].parts[0]
    assert isinstance(part, ToolReturnPart)
    assert isinstance(part.content, dict)
    assert "artifact_id" in part.content


def test_sanitize_history_leaves_small_and_non_vsh_returns() -> None:
    store = MemoryArtifactStore()
    small = {"ok": True}
    request = ModelRequest(
        parts=[
            ToolReturnPart(tool_name="vsh_search", content=small, tool_call_id="s1"),
            ToolReturnPart(tool_name="other", content=small, tool_call_id="o1"),
        ]
    )
    sanitized = _artifact_spill.sanitize_history_tool_returns(
        [request],
        store,
        threshold=10_000,
    )
    first_part = sanitized[0].parts[0]
    assert isinstance(first_part, ToolReturnPart)
    assert first_part.content == small


def test_spill_threshold_uses_deps_override() -> None:
    deps = VshAgentDeps(workspace_root="/tmp", artifact_spill_bytes=4096)
    assert _artifact_spill.spill_threshold(deps) == 4096


def test_vsh_capability_before_model_request_sanitizes_history(tmp_path: Path) -> None:
    import asyncio

    from pydantic_ai.models import ModelRequestParameters
    from pydantic_ai.models.test import TestModel

    store = MemoryArtifactStore()
    deps = VshAgentDeps(
        workspace_root=str(tmp_path),
        artifact_store=store,
        artifact_spill_bytes=32,
    )
    capability = VshCapability(tmp_path)
    model = TestModel()
    ctx = cast(
        RunContext[VshAgentDeps],
        RunContext(deps=deps, model=model, usage=None, prompt=None, run_step=0),  # type: ignore[arg-type]
    )
    large = {"blob": "q" * 256}
    request = ModelRequest(
        parts=[ToolReturnPart(tool_name="vsh_simulate", content=large, tool_call_id="sim-1")]
    )
    request_context = ModelRequestContext(
        messages=[request],
        model=model,
        model_settings=None,
        model_request_parameters=ModelRequestParameters(),
    )

    async def run_hook() -> ModelRequestContext:
        return await capability.before_model_request(ctx, request_context)

    updated = asyncio.run(run_hook())
    part = updated.messages[0].parts[0]
    assert isinstance(part, ToolReturnPart)
    assert isinstance(part.content, dict)
    assert "artifact_id" in part.content


def test_vsh_capability_after_tool_execute_spills(tmp_path: Path) -> None:
    import asyncio

    from pydantic_ai.messages import ToolCallPart
    from pydantic_ai.tools import ToolDefinition

    store = MemoryArtifactStore()
    deps = VshAgentDeps(
        workspace_root=str(tmp_path),
        artifact_store=store,
        artifact_spill_bytes=32,
    )
    capability = VshCapability(tmp_path)
    ctx = cast(
        RunContext[VshAgentDeps],
        RunContext(deps=deps, model=None, usage=None, prompt=None, run_step=0),  # type: ignore[arg-type]
    )
    large = {"blob": "z" * 256}

    async def run_hook() -> Any:
        return await capability.after_tool_execute(
            ctx,
            call=ToolCallPart(tool_name="vsh_simulate", args={}, tool_call_id="sim-1"),
            tool_def=ToolDefinition(
                name="vsh_simulate",
                description="",
                parameters_json_schema={},
            ),
            args={},
            result=large,
        )

    spilled = cast(dict[str, Any], asyncio.run(run_hook()))
    assert "artifact_id" in spilled
    assert store.get(spilled["artifact_id"]).ref.tool_name == "vsh_simulate"


def test_vsh_simulate_merges_execution_reason_kwarg(tmp_path: Path) -> None:
    from vsh.mcp import tools as mcp_tools
    from vsh.snapshot.builder import snapshot_workspace

    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = mcp_tools.simulate(
        "vsh_touch",
        snapshot.snapshot_id,
        {"path": "new.txt", "execution_reason": "create marker file"},
    )
    assert result["command"]["execution_reason"] == "create marker file"
