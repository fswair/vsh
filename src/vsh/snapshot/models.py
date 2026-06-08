from __future__ import annotations as _annotations

from pydantic import BaseModel, ConfigDict, Field

from vsh.session import SessionState


class SnapshotNode(BaseModel):
    model_config = ConfigDict(extra="forbid")

    path: str
    parent: str | None
    kind: str
    children: list[str] = Field(default_factory=list)
    size: int | None = None
    mode: int | None = None
    mtime_ns: int | None = None
    content_ref: str | None = None
    revision: int = 0


class WorkspaceSnapshot(BaseModel):
    model_config = ConfigDict(extra="forbid")

    snapshot_id: str
    session: SessionState
    generated_at_ns: int
    nodes: dict[str, SnapshotNode]
