from __future__ import annotations as _annotations

import json
import os
from dataclasses import dataclass, field
from pathlib import Path

from . import _common
from .models import ArtifactId, ArtifactIndexEntry, ArtifactRecord, normalize_artifact_id

__all__ = ("FileArtifactStore",)


def _extension_for_content_type(content_type: str) -> str:
    if content_type.startswith("application/json"):
        return "json"
    if content_type.startswith("text/"):
        return "txt"
    return "bin"


@dataclass
class FileArtifactStore:
    """Filesystem-backed artifact store under ``$VSH_DATA_DIR/artifacts/tool_outputs/``."""

    root: Path
    _records: dict[str, ArtifactRecord] = field(default_factory=dict, init=False, repr=False)

    def __post_init__(self) -> None:
        self.root.mkdir(parents=True, exist_ok=True)
        self._load_existing()

    def _payload_path(self, tool_name: str, artifact_id: str, *, content_type: str) -> Path:
        ext = _extension_for_content_type(content_type)
        return self.root / f"{tool_name}_{artifact_id}.{ext}"

    def _manifest_path(self, tool_name: str, artifact_id: str) -> Path:
        return self.root / f"{tool_name}_{artifact_id}.manifest.json"

    def _load_existing(self) -> None:
        for manifest_path in self.root.glob("*.manifest.json"):
            payload = json.loads(manifest_path.read_text(encoding="utf-8"))
            record = ArtifactRecord.model_validate(payload)
            self._records[record.ref.artifact_id] = record

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
        payload_path = self._payload_path(tool_name, artifact_id, content_type=content_type)
        manifest_path = self._manifest_path(tool_name, artifact_id)
        payload_path.write_bytes(payload)
        manifest_path.write_text(
            json.dumps(record.model_dump(mode="json"), indent=2, sort_keys=True),
            encoding="utf-8",
        )
        self._records[artifact_id] = record
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
        record = self.get(artifact_id)
        payload_path = self._payload_path(
            record.ref.tool_name,
            record.ref.artifact_id,
            content_type=record.ref.content_type,
        )
        if not payload_path.exists():
            msg = f"artifact payload missing: {record.ref.artifact_id}"
            raise KeyError(msg)
        payload = payload_path.read_bytes()
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
        record = self.get(artifact_id)
        updated = record.model_copy(update={"title": title, "tags": list(tags)})
        manifest_path = self._manifest_path(record.ref.tool_name, record.ref.artifact_id)
        manifest_path.write_text(
            json.dumps(updated.model_dump(mode="json"), indent=2, sort_keys=True),
            encoding="utf-8",
        )
        self._records[record.ref.artifact_id] = updated
        return updated

    def search(self, query: str) -> list[ArtifactIndexEntry]:
        return [
            _common.to_index_entry(record)
            for record in self._records.values()
            if _common.matches_search_query(record, query)
        ]


def default_filesystem_root() -> Path:
    data_dir = Path(os.environ.get("VSH_DATA_DIR", Path.home() / ".vsh"))
    return data_dir / "artifacts" / "tool_outputs"
