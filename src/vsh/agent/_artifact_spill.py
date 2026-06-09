from __future__ import annotations as _annotations

from dataclasses import replace
from typing import Any

from pydantic_ai.messages import ModelMessage, ModelRequest, ToolReturnPart

from vsh.artifacts import ArtifactStore
from vsh.artifacts._common import encode_tool_result

__all__ = (
    "ARTIFACT_PASSTHROUGH_TOOLS",
    "is_spillable_vsh_tool",
    "maybe_spill_tool_result",
    "sanitize_history_tool_returns",
    "spill_threshold",
)

ARTIFACT_PASSTHROUGH_TOOLS = frozenset(
    {
        "vsh_get_artifact",
        "vsh_index_artifact",
        "vsh_search_artifacts",
    }
)

_CODEMODE_MCP_SPILL_TOOLS = frozenset(
    {
        "search",
        "get_schema",
        "simulate",
        "vsh_sandbox",
        "apply",
        "apply_batch",
    }
)


def is_spillable_vsh_tool(tool_name: str) -> bool:
    if tool_name in ARTIFACT_PASSTHROUGH_TOOLS:
        return False
    if tool_name.startswith("vsh_"):
        return True
    return tool_name in _CODEMODE_MCP_SPILL_TOOLS


def spill_threshold(deps: Any) -> int:
    override = getattr(deps, "artifact_spill_bytes", None)
    if isinstance(override, int) and override > 0:
        return override
    from vsh.artifacts.factory import artifact_spill_bytes

    return artifact_spill_bytes()


def maybe_spill_tool_result(
    store: ArtifactStore,
    *,
    tool_name: str,
    result: Any,
    threshold: int,
    source_tool_call_id: str | None = None,
    plan_id: str | None = None,
) -> Any:
    if not is_spillable_vsh_tool(tool_name):
        return result
    if isinstance(result, dict) and "artifact_id" in result and "content_hash" in result:
        return result
    payload, content_type = encode_tool_result(result)
    if len(payload) <= threshold:
        return result
    record = store.put(
        tool_name=tool_name,
        payload=payload,
        content_type=content_type,
        source_tool_call_id=source_tool_call_id,
        plan_id=plan_id,
    )
    return record.ref.model_dump()


def sanitize_history_tool_returns(
    messages: list[ModelMessage],
    store: ArtifactStore,
    *,
    threshold: int,
) -> list[ModelMessage]:
    sanitized: list[ModelMessage] = []
    for message in messages:
        if not isinstance(message, ModelRequest):
            sanitized.append(message)
            continue
        new_parts: list[Any] = []
        changed = False
        for part in message.parts:
            if not isinstance(part, ToolReturnPart):
                new_parts.append(part)
                continue
            if not is_spillable_vsh_tool(part.tool_name):
                new_parts.append(part)
                continue
            spilled = maybe_spill_tool_result(
                store,
                tool_name=part.tool_name,
                result=part.content,
                threshold=threshold,
                source_tool_call_id=part.tool_call_id,
            )
            if spilled is part.content:
                new_parts.append(part)
            else:
                changed = True
                new_parts.append(replace(part, content=spilled))
        if changed:
            sanitized.append(replace(message, parts=new_parts))
        else:
            sanitized.append(message)
    return sanitized
