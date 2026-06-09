from __future__ import annotations as _annotations

import json
import os
from pathlib import Path
from typing import Any

from filelock import FileLock

from vsh.io.atomic import atomic_write_text
from vsh.plans.models import PlanRecord, SimulationResult
from vsh.registry import registrations
from vsh.schemas import StructuredCommand
from vsh.snapshot.models import WorkspaceSnapshot

__all__ = ("PersistenceStore", "persistence_enabled", "persistence_store")


def persistence_enabled() -> bool:
    return os.environ.get("VSH_PERSIST", "1") != "0"


class PersistenceStore:
    def __init__(self, root: Path | None = None) -> None:
        self.root = root or Path(os.environ.get("VSH_DATA_DIR", Path.home() / ".vsh"))

    def save_snapshot(self, snapshot: WorkspaceSnapshot) -> Path:
        directory = self.root / "snapshots"
        directory.mkdir(parents=True, exist_ok=True)
        path = directory / f"{snapshot.snapshot_id}.json"
        payload = json.dumps(snapshot.model_dump(mode="json"), indent=2, sort_keys=True)
        with FileLock(str(path) + ".lock"):
            atomic_write_text(path, payload)
        return path

    def load_snapshot(self, snapshot_id: str) -> WorkspaceSnapshot:
        path = self.root / "snapshots" / f"{snapshot_id}.json"
        payload = json.loads(path.read_text(encoding="utf-8"))
        return WorkspaceSnapshot.model_validate(payload)

    def save_plan(self, record: PlanRecord) -> Path:
        directory = self.root / "plans"
        directory.mkdir(parents=True, exist_ok=True)
        path = directory / f"{record.plan_id}.json"
        payload = record.model_dump(mode="json")
        payload["result"]["command_model_name"] = type(record.result.command).__name__
        text = json.dumps(payload, indent=2, sort_keys=True)
        with FileLock(str(path) + ".lock"):
            atomic_write_text(path, text)
        return path

    def load_plan(self, plan_id: str) -> PlanRecord:
        path = self.root / "plans" / f"{plan_id}.json"
        payload = json.loads(path.read_text(encoding="utf-8"))
        return _plan_record_from_payload(payload)


persistence_store = PersistenceStore()


def _plan_record_from_payload(payload: dict[str, Any]) -> PlanRecord:
    result_payload = dict(payload["result"])
    command_model_name = result_payload.pop("command_model_name", None)
    command_payload = result_payload.pop("command")
    command = _restore_command(command_model_name, command_payload)
    result = SimulationResult.model_validate({**result_payload, "command": command})
    return PlanRecord.model_validate({**payload, "result": result})


def _restore_command(model_name: str | None, payload: dict[str, Any]) -> StructuredCommand:
    if model_name is None:
        msg = "persisted plan is missing command_model_name"
        raise ValueError(msg)
    for registration in registrations.values():
        if registration.schema_model.__name__ == model_name:
            return registration.schema_model.model_validate(payload)
    msg = f"unknown persisted command model: {model_name}"
    raise ValueError(msg)
