from __future__ import annotations as _annotations

from pathlib import Path
from typing import Any

from pydantic_ai import Agent
from pydantic_ai.capabilities import Capability
from pydantic_ai.messages import ToolCallPart
from pydantic_ai.models import ModelRequestContext
from pydantic_ai.tools import RunContext, ToolDefinition

from vsh.artifacts import ArtifactStore, create_artifact_store

from . import _artifact_spill
from .deps import VshAgentDeps
from .toolset import _VSH_TOOLSET_INSTRUCTIONS, register_vsh_tools

__all__ = (
    "VshCapability",
    "create_vsh_agent",
)

_VSH_CAPABILITY_DESCRIPTION = (
    "Discover, simulate, approve, and execute structured workspace commands "
    "through the vsh validation-first flow."
)


class VshCapability(Capability[VshAgentDeps]):
    """pydantic-ai capability bundling vsh instructions and tools.

    Holds a [`VshAgentDeps`][vsh.agent.VshAgentDeps] instance for workspace runtime
    state. Pass ``vsh.deps`` to ``agent.run()`` — pydantic-ai does not wire deps
    from capabilities automatically.

    Set ``defer_loading=True`` to hide the workflow until the model calls
    ``load_capability``.
    """

    def __init__(
        self,
        workspace_root: str | Path,
        *,
        artifact_store: ArtifactStore | None = None,
        artifact_spill_bytes: int | None = None,
        id: str = "vsh",
        defer_loading: bool = False,
        description: str | None = None,
    ) -> None:
        self._deps = VshAgentDeps(
            workspace_root=str(Path(workspace_root).resolve()),
            artifact_store=artifact_store or create_artifact_store(),
            artifact_spill_bytes=artifact_spill_bytes,
        )
        super().__init__(
            id=id,
            description=description or _VSH_CAPABILITY_DESCRIPTION,
            instructions=_VSH_TOOLSET_INSTRUCTIONS,
            defer_loading=defer_loading,
        )
        register_vsh_tools(self)

    @property
    def deps(self) -> VshAgentDeps:
        """Runtime state shared across vsh tool calls for this workspace."""
        return self._deps

    @property
    def workspace_root(self) -> str:
        return self._deps.workspace_root

    async def after_tool_execute(
        self,
        ctx: RunContext[VshAgentDeps],
        *,
        call: ToolCallPart,
        tool_def: ToolDefinition,
        args: dict[str, Any],
        result: Any,
    ) -> Any:
        plan_id = ctx.deps.last_plan_id if call.tool_name == "vsh_simulate" else None
        return _artifact_spill.maybe_spill_tool_result(
            ctx.deps.artifact_store,
            tool_name=call.tool_name,
            result=result,
            threshold=_artifact_spill.spill_threshold(ctx.deps),
            source_tool_call_id=call.tool_call_id,
            plan_id=plan_id,
        )

    async def before_model_request(
        self,
        ctx: RunContext[VshAgentDeps],
        request_context: ModelRequestContext,
    ) -> ModelRequestContext:
        request_context.messages = _artifact_spill.sanitize_history_tool_returns(
            request_context.messages,
            ctx.deps.artifact_store,
            threshold=_artifact_spill.spill_threshold(ctx.deps),
        )
        return request_context


def create_vsh_agent(
    model: Any,
    workspace_root: str | Path,
    *,
    vsh: VshCapability | None = None,
    artifact_store: ArtifactStore | None = None,
    artifact_spill_bytes: int | None = None,
    defer_loading: bool = False,
    **agent_kwargs: Any,
) -> tuple[Agent[VshAgentDeps, str], VshCapability]:
    """Build a pydantic-ai Agent with vsh wired through capabilities."""
    capability = vsh or VshCapability(
        workspace_root,
        artifact_store=artifact_store,
        artifact_spill_bytes=artifact_spill_bytes,
        defer_loading=defer_loading,
    )
    agent = Agent(
        model,
        deps_type=VshAgentDeps,
        capabilities=[capability],
        **agent_kwargs,
    )
    return agent, capability
