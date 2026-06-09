from __future__ import annotations as _annotations

from pathlib import Path
from typing import Any

from pydantic_ai import Agent
from pydantic_ai.capabilities import Capability

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
        id: str = "vsh",
        defer_loading: bool = False,
        description: str | None = None,
    ) -> None:
        self._deps = VshAgentDeps.from_path(workspace_root)
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


def create_vsh_agent(
    model: Any,
    workspace_root: str | Path,
    *,
    vsh: VshCapability | None = None,
    defer_loading: bool = False,
    **agent_kwargs: Any,
) -> tuple[Agent[VshAgentDeps, str], VshCapability]:
    """Build a pydantic-ai Agent with vsh wired through capabilities."""
    capability = vsh or VshCapability(workspace_root, defer_loading=defer_loading)
    agent = Agent(
        model,
        deps_type=VshAgentDeps,
        capabilities=[capability],
        **agent_kwargs,
    )
    return agent, capability
