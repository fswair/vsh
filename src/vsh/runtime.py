from __future__ import annotations as _annotations

from dataclasses import dataclass, field

from vsh.persistence import persistence_enabled, persistence_store
from vsh.plans.models import PlanRecord, SimulationResult
from vsh.snapshot.models import WorkspaceSnapshot


@dataclass(slots=True)
class RuntimeLedger:
    snapshots: dict[str, WorkspaceSnapshot] = field(default_factory=dict)
    latest_snapshot_id: str | None = None
    plans: dict[str, PlanRecord] = field(default_factory=dict)
    latest_plan_id: str | None = None

    def record_snapshot(self, snapshot: WorkspaceSnapshot) -> None:
        self.snapshots[snapshot.snapshot_id] = snapshot
        self.latest_snapshot_id = snapshot.snapshot_id
        if persistence_enabled():
            persistence_store.save_snapshot(snapshot)

    def get_snapshot(self, snapshot_id: str | None = None) -> WorkspaceSnapshot:
        target_id = snapshot_id or self.latest_snapshot_id
        if target_id is None:
            raise KeyError("no snapshot has been recorded")
        return self.snapshots[target_id]

    def record_plan(
        self,
        result: SimulationResult,
        snapshot_id: str,
        *,
        path_fingerprints: dict[str, str],
        plan_fingerprint: str,
    ) -> PlanRecord:
        from vsh.persistence import persistence_store
        from vsh.plans.store import plan_store

        record = plan_store.save(
            result,
            snapshot_id,
            path_fingerprints=path_fingerprints,
            plan_fingerprint=plan_fingerprint,
        )
        self.plans[record.plan_id] = record
        self.latest_plan_id = record.plan_id
        if persistence_enabled():
            persistence_store.save_plan(record)
        return record

    def get_plan(self, plan_id: str) -> PlanRecord:
        return self.plans[plan_id]


runtime = RuntimeLedger()
