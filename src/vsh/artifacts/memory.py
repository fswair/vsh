from __future__ import annotations as _annotations

from dataclasses import dataclass, field

from . import _common
from .models import ArtifactId, ArtifactIndexEntry, ArtifactRecord, normalize_artifact_id

__all__ = ("MemoryArtifactStore",)


@dataclass
class MemoryArtifactStore:
    """In-memory artifact store for tests and ephemeral runs."""

    _records: dict[str, ArtifactRecord] = field(default_factory=dict)
    _payloads: dict[str, bytes] = field(default_factory=dict)

    def put(
        self,
        *,
        tool_name: str,
        payload: bytes,
        content_type: str,
        source_tool_call_id: str | None = None,
        plan_id: str | None = None,
    ) -> ArtifactRecord:
        artifact_id = _common.new_artifact_id()
        record = _common.build_artifact_record(
            artifact_id=artifact_id,
            payload=payload,
            content_type=content_type,
            tool_name=tool_name,
            source_tool_call_id=source_tool_call_id,
            plan_id=plan_id,
        )
        self._records[artifact_id] = record
        self._payloads[artifact_id] = payload
        return record

    def get(self, artifact_id: ArtifactId) -> ArtifactRecord:
        normalized = normalize_artifact_id(artifact_id)
        record = self._records.get(normalized)
        if record is None:
            msg = f"artifact not found: {normalized}"
            raise KeyError(msg)
        return record

    def read_bytes(
        self,
        artifact_id: ArtifactId,
        *,
        offset: int = 0,
        limit: int | None = None,
    ) -> bytes:
        normalized = normalize_artifact_id(artifact_id)
        payload = self._payloads.get(normalized)
        if payload is None:
            msg = f"artifact not found: {normalized}"
            raise KeyError(msg)
        if offset < 0:
            msg = "offset must be non-negative"
            raise ValueError(msg)
        sliced = payload[offset:]
        if limit is not None:
            if limit < 0:
                msg = "limit must be non-negative"
                raise ValueError(msg)
            return sliced[:limit]
        return sliced

    def index(
        self,
        artifact_id: ArtifactId,
        *,
        title: str,
        tags: list[str],
    ) -> ArtifactRecord:
        normalized = normalize_artifact_id(artifact_id)
        record = self.get(normalized)
        updated = record.model_copy(update={"title": title, "tags": list(tags)})
        self._records[normalized] = updated
        return updated

    def search(self, query: str) -> list[ArtifactIndexEntry]:
        return [
            _common.to_index_entry(record)
            for record in self._records.values()
            if _common.matches_search_query(record, query)
        ]
