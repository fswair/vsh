from __future__ import annotations as _annotations

import threading
import time
import uuid
from dataclasses import dataclass, field

from .models import ApprovalToken, PlanRecord, SimulationResult


@dataclass
class PlanStore:
    plans: dict[str, PlanRecord] = field(default_factory=dict)
    _tokens: dict[str, PlanRecord] = field(default_factory=dict)
    _lock: threading.RLock = field(default_factory=threading.RLock)

    def save(
        self,
        result: SimulationResult,
        snapshot_id: str,
        *,
        path_fingerprints: dict[str, str],
        plan_fingerprint: str,
    ) -> PlanRecord:
        with self._lock:
            record = PlanRecord(
                plan_id=result.plan_id,
                snapshot_id=snapshot_id,
                result=result,
                created_at_ns=time.time_ns(),
                path_fingerprints=path_fingerprints,
                plan_fingerprint=plan_fingerprint,
            )
            self.plans[record.plan_id] = record
            return record

    def get(self, plan_id: str) -> PlanRecord:
        with self._lock:
            return self.plans[plan_id]

    def approve(self, plan_id: str) -> ApprovalToken:
        with self._lock:
            record = self.get(plan_id)
            token = ApprovalToken(
                token=f"approval_{uuid.uuid4().hex}",
                plan_id=plan_id,
                approved_at_ns=time.time_ns(),
            )
            record.approval_token = token
            self._tokens[token.token] = record
            return token

    def get_by_token(self, approval_token: str) -> PlanRecord:
        with self._lock:
            try:
                return self._tokens[approval_token]
            except KeyError as exc:
                raise KeyError(f"unknown approval token: {approval_token}") from exc

    def mark_executed(self, approval_token: str) -> PlanRecord:
        with self._lock:
            record = self.get_by_token(approval_token)
            record.executed_at_ns = time.time_ns()
            self._tokens.pop(approval_token, None)
            return record


plan_store = PlanStore()
