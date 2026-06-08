from __future__ import annotations as _annotations

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

__all__ = (
    "ActualEffects",
    "RevalidationReport",
)


class ActualEffects(BaseModel):
    model_config = ConfigDict(extra="forbid")

    reads: list[str] = Field(default_factory=list)
    creates: list[str] = Field(default_factory=list)
    updates: list[str] = Field(default_factory=list)
    deletes: list[str] = Field(default_factory=list)
    renames: list[tuple[str, str]] = Field(default_factory=list)
    cwd_after: str | None = None


class RevalidationReport(BaseModel):
    model_config = ConfigDict(extra="forbid")

    status: Literal["ok", "stale"] = "ok"
    drift_messages: list[str] = Field(default_factory=list)
    refreshed_paths: list[str] = Field(default_factory=list)
