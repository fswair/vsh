from __future__ import annotations

import asyncio
import importlib

import pytest
from pydantic_ai import Agent
from pydantic_ai.capabilities import AbstractCapability
from pydantic_ai.models.test import TestModel
from pydantic_ai.toolsets import FunctionToolset

from vsh import HookDecision, HookScope, RequestEvent
from vsh.pydantic_ai import VshCapability


def test_capability_exposes_native_vsh_filesystem_toolset(tmp_path) -> None:
    capability = VshCapability(tmp_path)

    assert isinstance(capability, AbstractCapability)
    toolset = capability.get_toolset()
    assert isinstance(toolset, FunctionToolset)
    assert set(toolset.tools) == {
        "vsh_copy",
        "vsh_glob",
        "vsh_list",
        "vsh_mkdir",
        "vsh_move",
        "vsh_patch",
        "vsh_read",
        "vsh_remove",
        "vsh_run",
        "vsh_search",
        "vsh_write",
    }
    assert "never JSON objects" in (toolset.tools["vsh_run"].description or "")


def test_capability_runs_real_write_and_read_transactions(tmp_path) -> None:
    capability = VshCapability(tmp_path)

    written = asyncio.run(capability.vsh_write("/workspace/hello.txt", "hello"))
    read = asyncio.run(capability.vsh_read("/workspace/hello.txt"))

    assert written.state == "committed"
    assert written.changed_paths == 1
    assert not written.requires_review
    assert read.result == "hello"
    assert (tmp_path / "hello.txt").read_text() == "hello"


def test_capability_returns_hook_feedback_without_new_transaction_state(tmp_path) -> None:
    capability = VshCapability(
        tmp_path,
        policy="strict",
        hook_handler=lambda _event: HookDecision.review(
            "Confirm that replacing the production manifest is intended."
        ),
    )

    result = asyncio.run(capability.vsh_write("/workspace/manifest.txt", "new"))

    assert result.state == "pending_approval"
    assert result.requires_review
    assert result.hook_verdict == "review"
    assert result.feedback == "Confirm that replacing the production manifest is intended."
    assert not (tmp_path / "manifest.txt").exists()


def test_capability_runs_through_a_real_pydantic_ai_agent(tmp_path) -> None:
    (tmp_path / "visible.txt").write_text("content")
    capability = VshCapability(tmp_path)
    agent = Agent(TestModel(call_tools=["vsh_list"]), capabilities=[capability])

    result = agent.run_sync("List the workspace through VSH.")

    assert "visible.txt" in result.output


def test_capability_filesystem_surface_is_end_to_end_and_atomic(tmp_path) -> None:
    capability = VshCapability(
        tmp_path,
        hook_handler=lambda _event: HookDecision.approve("fixture operation is expected"),
    )

    async def scenario() -> None:
        await capability.vsh_mkdir("/workspace/generated")
        await capability.vsh_write("/workspace/generated/a.txt", "first")
        await capability.vsh_write("/workspace/generated/a.txt", "!", append=True)
        await capability.vsh_copy(
            "/workspace/generated/a.txt",
            "/workspace/generated/b.txt",
        )
        await capability.vsh_move(
            "/workspace/generated/b.txt",
            "/workspace/generated/c.txt",
        )
        patched = await capability.vsh_patch("/workspace/generated/a.txt", "first", "updated")
        searched = await capability.vsh_search(
            "updated", path="/workspace/generated", case_sensitive=False, max_results=5
        )
        globbed = await capability.vsh_glob("*.txt", path="/workspace/generated", max_results=5)
        listed = await capability.vsh_list("/workspace/generated")
        atomic = await capability.vsh_run(
            "value = vsh_read('/workspace/generated/a.txt')\n(value, len(value))",
            "inspect generated output atomically",
        )
        encoded = await capability.vsh_run("b'raw'", "return binary evidence")
        await capability.vsh_remove("/workspace/generated/c.txt")
        await capability.vsh_remove("/workspace/generated", recursive=True)

        assert patched.result == 1
        assert searched.result == [
            {
                "column": 1,
                "line": 1,
                "path": "/workspace/generated/a.txt",
                "text": "updated!",
            }
        ]
        assert globbed.result == [
            "/workspace/generated/a.txt",
            "/workspace/generated/c.txt",
        ]
        assert listed.result == [
            "/workspace/generated/a.txt",
            "/workspace/generated/c.txt",
        ]
        assert atomic.result == ["updated!", 8]
        assert encoded.result == {"encoding": "base64", "data": "cmF3"}

    asyncio.run(scenario())
    assert not (tmp_path / "generated").exists()


def test_capability_hook_paths_preserve_policy_and_feedback(tmp_path) -> None:
    (tmp_path / "read.txt").write_text("safe")
    verdicts: list[str] = []

    def follow_policy(_event: RequestEvent) -> HookDecision:
        verdicts.append("called")
        return HookDecision.follow_policy()

    all_requests = VshCapability(
        tmp_path,
        hook_handler=follow_policy,
        hook_scope=HookScope.ALL_REQUESTS,
    )
    read = asyncio.run(all_requests.vsh_read("/workspace/read.txt"))

    assert read.state == "committed"
    assert read.hook_verdict == "follow_policy"
    assert read.feedback is None
    assert verdicts == ["called"]

    review_only = VshCapability(tmp_path, hook_handler=follow_policy)
    no_hook_read = asyncio.run(review_only.vsh_read("/workspace/read.txt"))
    assert no_hook_read.hook_verdict is None
    assert verdicts == ["called"]

    denied = asyncio.run(
        all_requests.vsh_run(
            "try:\n    vsh_read('/workspace/.env')\nexcept PermissionError:\n    pass\n'denied'",
            "probe a protected path",
        )
    )
    assert denied.state == "denied"
    assert verdicts == ["called"]


def test_capability_rejects_non_json_native_result() -> None:
    module = importlib.import_module("vsh.pydantic_ai")
    normalize = vars(module)["_json_value"]

    with pytest.raises(TypeError, match="cannot be sent to Pydantic AI"):
        normalize(object())
