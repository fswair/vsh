from __future__ import annotations as _annotations

import re
from typing import Annotated

from pydantic import BaseModel, ConfigDict, Field

__all__ = (
    "ARTIFACT_ID_PATTERN",
    "ArtifactId",
    "ArtifactIndexEntry",
    "ArtifactRecord",
    "ArtifactRef",
    "normalize_artifact_id",
)

ARTIFACT_ID_PATTERN = re.compile(r"^[0-9a-f]{8,16}$")
ArtifactId = Annotated[str, Field(pattern=r"^[0-9a-f]{8,16}$")]


def normalize_artifact_id(value: str) -> str:
    """Validate and normalize a hex artifact id."""
    normalized = value.strip().lower()
    if not ARTIFACT_ID_PATTERN.fullmatch(normalized):
        msg = f"invalid artifact_id {value!r}; expected lowercase hex (8-16 chars)"
        raise ValueError(msg)
    return normalized


class ArtifactRef(BaseModel):
    """Compact reference returned when a tool output is spilled to the artifact store."""

    model_config = ConfigDict(extra="forbid")

    artifact_id: ArtifactId
    content_hash: str = Field(description="SHA-256 hex digest of the stored payload bytes.")
    byte_size: int = Field(ge=0)
    content_type: str
    tool_name: str
    preview: str = Field(description="Short UTF-8 preview of the spilled payload.")
    spilled_at_ns: int = Field(ge=0)


class ArtifactRecord(BaseModel):
    """Full artifact metadata including optional search fields."""

    model_config = ConfigDict(extra="forbid")

    ref: ArtifactRef
    title: str | None = None
    tags: list[str] = Field(default_factory=list)
    source_tool_call_id: str | None = None
    plan_id: str | None = None


class ArtifactIndexEntry(BaseModel):
    """Search result row for artifact discovery."""

    model_config = ConfigDict(extra="forbid")

    artifact_id: ArtifactId
    tool_name: str
    title: str | None = None
    tags: list[str] = Field(default_factory=list)
    byte_size: int = Field(ge=0)
    preview: str = ""
