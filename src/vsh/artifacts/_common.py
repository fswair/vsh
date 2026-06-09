from __future__ import annotations as _annotations

import hashlib
import json
import secrets
import time
from typing import Any

from .models import ArtifactIndexEntry, ArtifactRecord, ArtifactRef, normalize_artifact_id

__all__ = (
    "build_artifact_ref",
    "content_preview",
    "new_artifact_id",
    "serialize_payload",
)


def new_artifact_id() -> str:
    return secrets.token_hex(8)


def serialize_payload(payload: bytes, *, content_type: str) -> tuple[bytes, str]:
    return payload, content_type


def content_hash(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def content_preview(payload: bytes, *, limit: int = 240) -> str:
    text = payload.decode("utf-8", errors="replace")
    if len(text) <= limit:
        return text
    return f"{text[:limit]}…"


def build_artifact_ref(
    *,
    artifact_id: str,
    payload: bytes,
    content_type: str,
    tool_name: str,
) -> ArtifactRef:
    return ArtifactRef(
        artifact_id=normalize_artifact_id(artifact_id),
        content_hash=content_hash(payload),
        byte_size=len(payload),
        content_type=content_type,
        tool_name=tool_name,
        preview=content_preview(payload),
        spilled_at_ns=time.time_ns(),
    )


def build_artifact_record(
    *,
    artifact_id: str,
    payload: bytes,
    content_type: str,
    tool_name: str,
    source_tool_call_id: str | None = None,
    plan_id: str | None = None,
) -> ArtifactRecord:
    return ArtifactRecord(
        ref=build_artifact_ref(
            artifact_id=artifact_id,
            payload=payload,
            content_type=content_type,
            tool_name=tool_name,
        ),
        source_tool_call_id=source_tool_call_id,
        plan_id=plan_id,
    )


def to_index_entry(record: ArtifactRecord) -> ArtifactIndexEntry:
    return ArtifactIndexEntry(
        artifact_id=record.ref.artifact_id,
        tool_name=record.ref.tool_name,
        title=record.title,
        tags=list(record.tags),
        byte_size=record.ref.byte_size,
        preview=record.ref.preview,
    )


def matches_search_query(record: ArtifactRecord, query: str) -> bool:
    normalized_query = query.strip().lower()
    if not normalized_query:
        return True
    if normalized_query == record.ref.artifact_id:
        return True
    if record.title is not None and normalized_query in record.title.lower():
        return True
    return any(normalized_query in tag.lower() for tag in record.tags)


def encode_tool_result(result: Any) -> tuple[bytes, str]:
    if isinstance(result, bytes):
        return result, "application/octet-stream"
    if isinstance(result, str):
        return result.encode("utf-8"), "text/plain; charset=utf-8"
    encoded = json.dumps(result, ensure_ascii=False, separators=(",", ":"), default=str).encode(
        "utf-8"
    )
    return encoded, "application/json; charset=utf-8"
