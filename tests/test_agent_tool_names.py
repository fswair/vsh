from __future__ import annotations as _annotations

import asyncio
from pathlib import Path

import pytest
from pydantic_ai.messages import ModelResponse, ToolCallPart
from pydantic_ai.models import ModelRequestContext, ModelRequestParameters
from pydantic_ai.models.test import TestModel

from vsh.agent import VshCapability
from vsh.agent._tool_names import normalize_agent_tool_name


@pytest.mark.parametrize(
    ("raw", "expected"),
    [
        ("vsh_search", "vsh_search"),
        ("default_api:vsh_search", "vsh_search"),
        ("some.namespace:vsh_simulate", "vsh_simulate"),
        ("default_api:other_tool", "default_api:other_tool"),
        ("not_a_tool", "not_a_tool"),
    ],
)
def test_normalize_agent_tool_name(raw: str, expected: str) -> None:
    assert normalize_agent_tool_name(raw) == expected


def test_vsh_capability_normalizes_prefixed_tool_calls_in_model_response(tmp_path: Path) -> None:
    from typing import cast

    from pydantic_ai.tools import RunContext

    from vsh.agent import VshAgentDeps

    capability = VshCapability(tmp_path)
    ctx = cast(
        RunContext[VshAgentDeps],
        RunContext(
            deps=capability.deps,
            model=TestModel(),
            usage=None,  # type: ignore[arg-type]
            prompt=None,
            run_step=0,
        ),
    )
    response = ModelResponse(
        parts=[
            ToolCallPart(
                tool_name="default_api:vsh_search",
                args={"query": "list"},
                tool_call_id="call-1",
            )
        ]
    )
    request_context = ModelRequestContext(
        messages=[],
        model=TestModel(),
        model_settings=None,
        model_request_parameters=ModelRequestParameters(),
    )

    async def run_hook() -> ModelResponse:
        return await capability.after_model_request(
            ctx,
            request_context=request_context,
            response=response,
        )

    updated = asyncio.run(run_hook())
    part = updated.parts[0]
    assert isinstance(part, ToolCallPart)
    assert part.tool_name == "vsh_search"
