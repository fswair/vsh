from __future__ import annotations as _annotations

from dataclasses import dataclass

from vsh.simulate.approval_levels import ApprovalTier
from vsh.simulate.models import PolicyDecision

from .models import PlanRecord

__all__ = (
    "ApprovalContext",
    "ApprovalDeniedError",
    "ApproveItem",
)


class ApprovalDeniedError(Exception):
    """Raised when an approval handler rejects plan approval."""

    def __init__(self, message: str, *, plan_id: str | None = None) -> None:
        super().__init__(message)
        self.plan_id = plan_id


@dataclass(frozen=True, kw_only=True)
class ApprovalContext:
    """Runtime context passed to approval handlers."""

    auto: bool = False


@dataclass(frozen=True, kw_only=True)
class ApproveItem:
    """Immutable approval request payload for handler callbacks."""

    plan_id: str
    snapshot_id: str
    workspace_root: str
    shell_preview: str
    decision: PolicyDecision
    approval_tier: ApprovalTier
    requires_manual_approval: bool
    execution_eligible: bool
    execution_eligibility_reason: str | None
    plan_fingerprint: str

    @classmethod
    def from_plan_record(cls, record: PlanRecord, *, workspace_root: str) -> ApproveItem:
        result = record.result
        return cls(
            plan_id=record.plan_id,
            snapshot_id=record.snapshot_id,
            workspace_root=workspace_root,
            shell_preview=result.shell_preview,
            decision=result.decision,
            approval_tier=result.approval_tier,
            requires_manual_approval=result.requires_manual_approval,
            execution_eligible=result.execution_eligible,
            execution_eligibility_reason=result.execution_eligibility_reason,
            plan_fingerprint=record.plan_fingerprint,
        )
