from __future__ import annotations as _annotations

from .models import ApprovalToken
from .store import plan_store


def approve_plan(plan_id: str) -> ApprovalToken:
    return plan_store.approve(plan_id)
