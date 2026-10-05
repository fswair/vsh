from __future__ import annotations

import asyncio
import importlib
from dataclasses import asdict

import pytest
from pydantic_ai import Agent
from pydantic_ai.capabilities import AbstractCapability
from pydantic_ai.messages import ModelMessage, ModelResponse, TextPart
from pydantic_ai.models.function import AgentInfo, FunctionModel
from pydantic_ai.models.test import TestModel
from pydantic_ai.toolsets import FunctionToolset

from vsh import BashConfig, HookDecision, HookScope, RequestEvent, VshRuntimeError
from vsh._response import response_size
from vsh.pydantic_ai import VshCapability


@pytest.mark.parametrize("bash_enabled", [False, True])
def test_agent_receives_language_specific_capability_instructions(tmp_path, bash_enabled) -> None:
    capability = VshCapability(tmp_path, bash=BashConfig() if bash_enabled else None)

    def answer(messages: list[ModelMessage], info: AgentInfo) -> ModelResponse:
        assert info.instructions is not None
        assert "For language='monty' (the default)" in info.instructions
        assert "If the host enables language='bash', pass bounded shell source" in info.instructions
        tool = next(tool for tool in info.function_tools if tool.name == "vsh_run")
        assert tool.parameters_json_schema["properties"]["language"]["enum"] == (
            ["monty", "bash"] if bash_enabled else ["monty"]
        )
        return ModelResponse(parts=[TextPart("instructions verified")])

    agent = Agent(FunctionModel(answer), capabilities=[capability])
    assert (
        agent.run_sync("Describe the enabled execution surface.").output == "instructions verified"
    )


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


def test_run_only_keeps_guest_functions_and_one_agent_tool(tmp_path) -> None:
    capability = VshCapability(tmp_path, run_only=True)
    toolset = capability.get_toolset()
    assert isinstance(toolset, FunctionToolset)
    assert set(toolset.tools) == {"vsh_run"}
    result = asyncio.run(
        capability.vsh_run(
            "vsh_write('/workspace/item.txt', 'value')\nvsh_read('/workspace/item.txt')",
            intent="write and verify one item",
        )
    )
    assert result.state == "committed"
    assert result.result == "value"


def test_capability_runs_real_write_and_read_transactions(tmp_path) -> None:
    capability = VshCapability(tmp_path)

    written = asyncio.run(capability.vsh_write("/workspace/hello.txt", "hello"))
    read = asyncio.run(capability.vsh_read("/workspace/hello.txt"))

    assert written.state == "committed"
    assert written.changed_paths == 1
    assert not written.requires_review
    assert read.result == "hello"
    assert (tmp_path / "hello.txt").read_text() == "hello"
    repeated = asyncio.run(capability.vsh_read("/workspace/hello.txt"))
    assert repeated.result == read.result
    assert repeated.transaction != read.transaction


@pytest.mark.parametrize("hooked", [False, True])
@pytest.mark.parametrize(
    "value",
    [
        "{1, 2}",
        "frozenset([1, 2])",
        "{1: 'one', '1': 'string'}",
        "float('nan')",
        "{'nested': [set([1])]} ",
    ],
)
def test_agent_result_validation_precedes_host_mutation(tmp_path, hooked, value) -> None:
    events = []

    def approve(event: RequestEvent) -> HookDecision:
        events.append(event)
        return HookDecision.approve("test")

    capability = VshCapability(
        tmp_path, hook_handler=approve if hooked else None, hook_scope=HookScope.ALL_REQUESTS
    )
    with pytest.raises(VshRuntimeError, match="agent JSON"):
        asyncio.run(
            capability.vsh_run(
                f"vsh_write('/workspace/created.txt', 'must not commit')\n{value}",
                intent="test result projection",
            )
        )
    assert not (tmp_path / "created.txt").exists()
    assert not events


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


def test_agent_response_budget_preserves_commit_outcome(tmp_path) -> None:
    capability = VshCapability(tmp_path, max_response_bytes=4096)
    result = asyncio.run(
        capability.vsh_run(
            "vsh_write('/workspace/ok.txt', 'ok')\nprint('🙂' * 10000)\n'large' * 10000",
            "bounded presentation",
        )
    )
    assert result.state == "committed" and result.response_truncated
    assert (tmp_path / "ok.txt").read_text() == "ok"
    assert response_size(asdict(result)) <= 4096
    assert result.result is None and result.stdout == ""

    reviewed = VshCapability(
        tmp_path,
        max_response_bytes=4096,
        hook_scope=HookScope.ALL_REQUESTS,
        hook_handler=lambda _: HookDecision.review("🙂" * 3000),
    )
    result = asyncio.run(reviewed.vsh_run("None", "bounded review feedback"))
    assert result.state == "pending_approval" and result.response_truncated
    assert response_size(asdict(result)) <= 4096
    assert result.feedback is not None and result.feedback.endswith("[feedback truncated]")
    with pytest.raises(ValueError, match="max_response_bytes"):
        VshCapability(tmp_path, max_response_bytes=1)


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
    with pytest.raises(TypeError, match="keys must be strings"):
        normalize({1: "integer", "1": "string"})


def test_bash_capability_opt_in_bytes_and_review_output_gate(tmp_path) -> None:
    (tmp_path / "binary.bin").write_bytes(b"\xff\x00")
    disabled = VshCapability(tmp_path)
    disabled_tools = disabled.get_toolset()
    assert isinstance(disabled_tools, FunctionToolset)
    assert disabled_tools.tools["vsh_run"].function_schema.json_schema["properties"]["language"][
        "enum"
    ] == ["monty"]
    with pytest.raises(ValueError, match="not enabled"):
        asyncio.run(disabled.vsh_run("true", "test host authorization", language="bash"))

    enabled = VshCapability(tmp_path, bash=BashConfig())
    enabled_tools = enabled.get_toolset()
    assert isinstance(enabled_tools, FunctionToolset)
    assert enabled_tools.tools["vsh_run"].function_schema.json_schema["properties"]["language"][
        "enum"
    ] == ["monty", "bash"]
    result = asyncio.run(
        enabled.vsh_run(
            "cat binary.bin; printf ok > result.txt", "write a verified result", language="bash"
        )
    )
    assert result.state == "committed" and result.language == "bash"
    assert result.result == {
        "exit_code": 0,
        "profile": "vsh-bash-bounded-v4",
        "stdout": {"encoding": "base64", "data": "/wA="},
        "stderr": {"encoding": "base64", "data": ""},
    }
    assert (tmp_path / "result.txt").read_text() == "ok"

    reviewed = VshCapability(
        tmp_path,
        bash=BashConfig(),
        hook_handler=lambda _event: HookDecision.review("verify recipient"),
        hook_scope=HookScope.ALL_REQUESTS,
    )
    withheld = asyncio.run(
        reviewed.vsh_run("cat binary.bin; printf sensitive >&2", "inspect a file", language="bash")
    )
    assert withheld.state == "pending_approval" and withheld.feedback == "verify recipient"
    assert withheld.result is None and withheld.stdout == "" and withheld.stderr == ""
