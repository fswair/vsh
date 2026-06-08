from __future__ import annotations as _annotations

from vsh.extensions.registry import extensions
from vsh.runtime import runtime

from .approval_models import ApprovalContext, ApproveItem
from .models import PlanRecord

__all__ = ("run_approval_handlers",)


def run_approval_handlers(record: PlanRecord, *, auto: bool) -> None:
    """Invoke registered approval handlers before minting an approval token."""
    if not extensions.approval_handlers:
        return
    snapshot = runtime.get_snapshot(record.snapshot_id)
    item = ApproveItem.from_plan_record(
        record,
        workspace_root=snapshot.session.workspace_root,
    )
    ctx = ApprovalContext(auto=auto)
    for handler in extensions.approval_handlers:
        handler(ctx, item)
