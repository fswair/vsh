from __future__ import annotations as _annotations

import json
from pathlib import Path

import pytest

from vsh.artifacts import (
    FileArtifactStore,
    MemoryArtifactStore,
    artifact_spill_bytes,
    create_artifact_store,
    normalize_artifact_id,
)
from vsh.artifacts._common import encode_tool_result, matches_search_query
from vsh.artifacts.factory import _persistence_enabled
from vsh.artifacts.models import ArtifactRecord, ArtifactRef


def test_normalize_artifact_id_accepts_hex() -> None:
    assert normalize_artifact_id("A1B2C3D4E5F60708") == "a1b2c3d4e5f60708"


def test_normalize_artifact_id_rejects_invalid() -> None:
    with pytest.raises(ValueError, match="invalid artifact_id"):
        normalize_artifact_id("not-hex")


def test_memory_store_put_get_read_index_search() -> None:
    store = MemoryArtifactStore()
    payload = b'{"hello":"world"}'
    record = store.put(
        tool_name="vsh_simulate",
        payload=payload,
        content_type="application/json",
        source_tool_call_id="call-1",
        plan_id="plan_1",
    )
    assert record.ref.tool_name == "vsh_simulate"
    assert store.get(record.ref.artifact_id).ref.byte_size == len(payload)
    assert store.read_bytes(record.ref.artifact_id) == payload
    assert store.read_bytes(record.ref.artifact_id, offset=2, limit=3) == payload[2:5]
    indexed = store.index(record.ref.artifact_id, title="Sim output", tags=["sim"])
    assert indexed.title == "Sim output"
    assert indexed.tags == ["sim"]
    hits = store.search("sim")
    assert len(hits) == 1
    assert hits[0].artifact_id == record.ref.artifact_id


def test_memory_store_missing_and_invalid_ranges() -> None:
    store = MemoryArtifactStore()
    with pytest.raises(KeyError, match="artifact not found"):
        store.get("abcd1234abcd1234")
    record = store.put(
        tool_name="vsh_simulate",
        payload=b"x",
        content_type="text/plain",
    )
    with pytest.raises(ValueError, match="offset must be non-negative"):
        store.read_bytes(record.ref.artifact_id, offset=-1)
    with pytest.raises(ValueError, match="limit must be non-negative"):
        store.read_bytes(record.ref.artifact_id, limit=-1)


def test_filesystem_store_round_trip(tmp_path: Path) -> None:
    root = tmp_path / "artifacts"
    store = FileArtifactStore(root=root)
    record = store.put(
        tool_name="vsh_sandbox",
        payload=b"large-output",
        content_type="text/plain; charset=utf-8",
    )
    reloaded = FileArtifactStore(root=root)
    assert reloaded.get(record.ref.artifact_id).ref.preview
    assert reloaded.read_bytes(record.ref.artifact_id) == b"large-output"
    updated = reloaded.index(record.ref.artifact_id, title="sandbox", tags=["batch"])
    assert updated.title == "sandbox"
    assert reloaded.search(record.ref.artifact_id)[0].artifact_id == record.ref.artifact_id


def test_filesystem_store_missing_payload(tmp_path: Path) -> None:
    root = tmp_path / "artifacts"
    store = FileArtifactStore(root=root)
    record = store.put(tool_name="vsh_simulate", payload=b"x", content_type="text/plain")
    payload_path = root / f"vsh_simulate_{record.ref.artifact_id}.txt"
    payload_path.unlink()
    with pytest.raises(KeyError, match="artifact payload missing"):
        store.read_bytes(record.ref.artifact_id)


def test_create_artifact_store_respects_env(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("VSH_ARTIFACT_STORE", "memory")
    monkeypatch.setenv("VSH_PERSIST", "1")
    assert isinstance(create_artifact_store(), MemoryArtifactStore)
    monkeypatch.delenv("VSH_ARTIFACT_STORE")
    monkeypatch.setenv("VSH_PERSIST", "0")
    assert isinstance(create_artifact_store(), MemoryArtifactStore)


def test_artifact_spill_bytes_invalid_env(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("VSH_ARTIFACT_SPILL_BYTES", "nope")
    assert artifact_spill_bytes() == 8192


def test_encode_tool_result_variants() -> None:
    assert encode_tool_result(b"raw") == (b"raw", "application/octet-stream")
    text_bytes, text_type = encode_tool_result("hello")
    assert text_bytes == b"hello"
    assert text_type.startswith("text/plain")
    json_bytes, json_type = encode_tool_result({"a": 1})
    assert json.loads(json_bytes) == {"a": 1}
    assert json_type.startswith("application/json")


def test_matches_search_query_cases() -> None:
    record = ArtifactRecord(
        ref=ArtifactRef(
            artifact_id="abcd1234abcd1234",
            content_hash="hash",
            byte_size=1,
            content_type="text/plain",
            tool_name="vsh_simulate",
            preview="p",
            spilled_at_ns=1,
        ),
        title="Plan output",
        tags=["sim"],
    )
    assert matches_search_query(record, "")
    assert matches_search_query(record, "abcd1234abcd1234")
    assert matches_search_query(record, "plan")
    assert matches_search_query(record, "sim")
    assert not matches_search_query(record, "missing")


def test_persistence_enabled_helper(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("VSH_PERSIST", "0")
    assert _persistence_enabled() is False


def test_create_artifact_store_filesystem_backend(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    monkeypatch.setenv("VSH_PERSIST", "1")
    monkeypatch.delenv("VSH_ARTIFACT_STORE", raising=False)
    monkeypatch.setenv("VSH_DATA_DIR", str(tmp_path))
    from vsh.artifacts.factory import create_artifact_store, default_artifact_store

    default_artifact_store.cache_clear()  # type: ignore[attr-defined]
    store = create_artifact_store()
    assert isinstance(store, FileArtifactStore)
    assert store.root == tmp_path / "artifacts" / "tool_outputs"
    assert isinstance(default_artifact_store(), FileArtifactStore)


def test_filesystem_store_extension_and_missing_record(tmp_path: Path) -> None:
    from vsh.artifacts.filesystem import (
        FileArtifactStore,
        _extension_for_content_type,
        default_filesystem_root,
    )

    assert _extension_for_content_type("application/json; charset=utf-8") == "json"
    assert _extension_for_content_type("text/plain") == "txt"
    assert _extension_for_content_type("application/octet-stream") == "bin"
    root = tmp_path / "artifacts"
    store = FileArtifactStore(root=root)
    json_record = store.put(
        tool_name="vsh_simulate",
        payload=b"{}",
        content_type="application/json",
    )
    bin_record = store.put(
        tool_name="vsh_sandbox",
        payload=b"\x00\x01",
        content_type="application/octet-stream",
    )
    assert json_record.ref.artifact_id
    assert bin_record.ref.artifact_id
    with pytest.raises(KeyError, match="artifact not found"):
        store.get("abcd1234abcd1234")
    with pytest.raises(ValueError, match="offset must be non-negative"):
        store.read_bytes(json_record.ref.artifact_id, offset=-1)
    with pytest.raises(ValueError, match="limit must be non-negative"):
        store.read_bytes(json_record.ref.artifact_id, limit=-1)
    assert store.read_bytes(json_record.ref.artifact_id, offset=0, limit=1) == b"{"
    monkeypatch_root = tmp_path / "data"
    monkeypatch_root.mkdir()
    import os

    os.environ["VSH_DATA_DIR"] = str(monkeypatch_root)
    assert default_filesystem_root() == monkeypatch_root / "artifacts" / "tool_outputs"


def test_memory_store_read_bytes_missing_payload() -> None:
    store = MemoryArtifactStore()
    record = store.put(tool_name="vsh_simulate", payload=b"x", content_type="text/plain")
    del store._payloads[record.ref.artifact_id]  # noqa: SLF001
    with pytest.raises(KeyError, match="artifact not found"):
        store.read_bytes(record.ref.artifact_id)


def test_serialize_payload_passthrough() -> None:
    from vsh.artifacts._common import serialize_payload

    assert serialize_payload(b"abc", content_type="text/plain") == (b"abc", "text/plain")
