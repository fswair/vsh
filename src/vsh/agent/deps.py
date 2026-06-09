from __future__ import annotations as _annotations

from dataclasses import dataclass, field
from pathlib import Path

from vsh.artifacts import ArtifactStore, create_artifact_store

__all__ = ("VshAgentDeps",)


@dataclass(kw_only=True)
class VshAgentDeps:
    """Runtime state shared across vsh pydantic-ai tool calls."""

    workspace_root: str
    snapshot_id: str | None = None
    last_plan_id: str | None = None
    last_approval_token: str | None = None
    artifact_store: ArtifactStore = field(default_factory=create_artifact_store)
    artifact_spill_bytes: int | None = None

    @classmethod
    def from_path(cls, workspace_root: str | Path) -> VshAgentDeps:
        return cls(workspace_root=str(Path(workspace_root).resolve()))
