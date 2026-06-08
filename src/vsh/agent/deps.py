from __future__ import annotations as _annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(kw_only=True)
class VshAgentDeps:
    """Runtime state shared across vsh pydantic-ai tool calls."""

    workspace_root: str
    snapshot_id: str | None = None
    last_plan_id: str | None = None
    last_approval_token: str | None = None

    @classmethod
    def from_path(cls, workspace_root: str | Path) -> VshAgentDeps:
        return cls(workspace_root=str(Path(workspace_root).resolve()))
