from __future__ import annotations as _annotations

from pathlib import Path

from vsh.persistence import PersistenceStore
from vsh.plans.store import plan_store
from vsh.schemas import LsCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_persistence_store_round_trips_snapshot_and_plan(tmp_path: Path) -> None:
    store = PersistenceStore(root=tmp_path / "data")
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(LsCommand(path="."), snapshot)
    record = plan_store.get(result.plan_id)

    snapshot_path = store.save_snapshot(snapshot)
    plan_path = store.save_plan(record)
    loaded_snapshot = store.load_snapshot(snapshot.snapshot_id)
    loaded_plan = store.load_plan(record.plan_id)

    assert snapshot_path.exists()
    assert plan_path.exists()
    assert loaded_snapshot.snapshot_id == snapshot.snapshot_id
    assert loaded_plan.plan_id == record.plan_id
    assert loaded_plan.plan_fingerprint == record.plan_fingerprint
