from __future__ import annotations as _annotations

from typing import Protocol

from .models import ArtifactId, ArtifactIndexEntry, ArtifactRecord

__all__ = ("ArtifactStore",)


class ArtifactStore(Protocol):
    """Storage backend for spilled tool outputs."""

    def put(
        self,
        *,
        tool_name: str,
        payload: bytes,
        content_type: str,
        source_tool_call_id: str | None = None,
        plan_id: str | None = None,
    ) -> ArtifactRecord: ...

    def get(self, artifact_id: ArtifactId) -> ArtifactRecord: ...

    def read_bytes(
        self,
        artifact_id: ArtifactId,
        *,
        offset: int = 0,
        limit: int | None = None,
    ) -> bytes: ...

    def index(
        self,
        artifact_id: ArtifactId,
        *,
        title: str,
        tags: list[str],
    ) -> ArtifactRecord: ...

    def search(self, query: str) -> list[ArtifactIndexEntry]: ...
