from __future__ import annotations as _annotations

from .models import ApprovalToken
from .store import plan_store

__all__ = ("approve_plan", "auto_approve_plan")


def approve_plan(plan_id: str, *, auto: bool = False) -> ApprovalToken:
    """Approve a simulated plan. Use auto=True only for read-only auto-approvable plans."""
    record = plan_store.get(plan_id)
    if auto and record.result.requires_manual_approval:
        msg = f"plan {plan_id} requires manual approval (tier={record.result.approval_tier})"
        raise ValueError(msg)
    if not record.result.execution_eligible:
        reason = record.result.execution_eligibility_reason or "plan is not execution-eligible"
        raise ValueError(f"plan not eligible for approval: {reason}")
    return plan_store.approve(plan_id)


def auto_approve_plan(plan_id: str) -> ApprovalToken:
    """Approve a read-only plan without requiring manual approval."""
    return approve_plan(plan_id, auto=True)
