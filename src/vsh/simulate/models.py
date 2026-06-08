from __future__ import annotations as _annotations

from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

PolicyDecision = Literal["approve", "reject", "approve_with_warning"]


class Overlay(BaseModel):
    model_config = ConfigDict(extra="forbid")

    created: dict[str, dict[str, object]] = Field(default_factory=dict)
    updated: dict[str, dict[str, object]] = Field(default_factory=dict)
    deleted: set[str] = Field(default_factory=set)
    renames: list[tuple[str, str]] = Field(default_factory=list)
    cwd_override: str | None = None


class AccessJournal(BaseModel):
    model_config = ConfigDict(extra="forbid")

    metadata_reads: set[str] = Field(default_factory=set)
    content_reads: set[str] = Field(default_factory=set)
    creates: set[str] = Field(default_factory=set)
    deletes: set[str] = Field(default_factory=set)
    metadata_writes: set[str] = Field(default_factory=set)
    content_writes: set[str] = Field(default_factory=set)
    renames: list[tuple[str, str]] = Field(default_factory=list)
    cwd_changes: list[str] = Field(default_factory=list)


class PredictedEffects(BaseModel):
    model_config = ConfigDict(extra="forbid")

    reads: list[str] = Field(default_factory=list)
    creates: list[str] = Field(default_factory=list)
    deletes: list[str] = Field(default_factory=list)
    updates: list[str] = Field(default_factory=list)
    renames: list[tuple[str, str]] = Field(default_factory=list)
    cwd_after: str | None = None
