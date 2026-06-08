from __future__ import annotations as _annotations

import time

from vsh.effects import ActualEffects
from vsh.execute.dispatch import ExecutionContext, apply_command, effects_match_prediction
from vsh.execute.revalidate import revalidate_plan
from vsh.extensions.registry import extensions
from vsh.persistence import persistence_enabled, persistence_store
from vsh.plans.models import ExecutionResult
from vsh.plans.store import plan_store
from vsh.runtime import runtime
from vsh.snapshot.models import WorkspaceSnapshot

__all__ = ("execute_approved",)


def execute_approved(approval_token: str) -> ExecutionResult:
    record = plan_store.get_by_token(approval_token)
    if record.approval_token is None:
        raise ValueError(f"plan not approved: {record.plan_id}")
    if not record.result.execution_eligible:
        reason = record.result.execution_eligibility_reason or "plan is not eligible for execution"
        raise ValueError(f"plan not eligible for execution: {reason}")
    if record.executed_at_ns is not None:
        raise ValueError(f"plan already executed: {record.plan_id}")

    snapshot = runtime.get_snapshot(record.snapshot_id)
    revalidation, snapshot = revalidate_plan(record, snapshot)
    if revalidation.status == "stale":
        runtime.record_snapshot(snapshot)
        if persistence_enabled():
            persistence_store.save_snapshot(snapshot)
        drift = "; ".join(revalidation.drift_messages)
        return ExecutionResult(
            plan_id=record.plan_id,
            approval_token=approval_token,
            shell_preview=record.result.shell_preview,
            decision=record.result.decision,
            execution_eligible=record.result.execution_eligible,
            applied=False,
            reason=f"plan is stale: {drift}",
            revalidation=revalidation,
            actual_effects=None,
            matches_prediction=None,
        )

    ctx = ExecutionContext(
        workspace_root=snapshot.session.workspace_root,
        cwd_logical=snapshot.session.cwd_logical,
    )
    try:
        actual_effects = apply_command(record.result.command, ctx)
    except (OSError, ValueError, FileNotFoundError) as exc:
        return ExecutionResult(
            plan_id=record.plan_id,
            approval_token=approval_token,
            shell_preview=record.result.shell_preview,
            decision=record.result.decision,
            execution_eligible=record.result.execution_eligible,
            applied=False,
            reason=str(exc),
            revalidation=revalidation,
            actual_effects=None,
            matches_prediction=None,
        )

    updated_snapshot: WorkspaceSnapshot = _apply_session_updates(snapshot, actual_effects)
    runtime.record_snapshot(updated_snapshot)
    if persistence_enabled():
        persistence_store.save_snapshot(updated_snapshot)

    _run_extension_hooks(updated_snapshot, actual_effects)

    record.executed_at_ns = time.time_ns()
    if persistence_enabled():
        persistence_store.save_plan(record)

    matches = effects_match_prediction(record.result.predicted_effects, actual_effects)
    return ExecutionResult(
        plan_id=record.plan_id,
        approval_token=approval_token,
        shell_preview=record.result.shell_preview,
        decision=record.result.decision,
        execution_eligible=record.result.execution_eligible,
        applied=True,
        reason=None,
        revalidation=revalidation,
        actual_effects=actual_effects,
        matches_prediction=matches,
    )


def _apply_session_updates(
    snapshot: WorkspaceSnapshot,
    actual_effects: ActualEffects,
) -> WorkspaceSnapshot:
    if actual_effects.cwd_after is None:
        return snapshot
    return snapshot.model_copy(
        update={"session": snapshot.session.with_cwd(actual_effects.cwd_after)}
    )


def _run_extension_hooks(snapshot: WorkspaceSnapshot, actual_effects: ActualEffects) -> None:
    touched = sorted(
        set(actual_effects.reads)
        | set(actual_effects.creates)
        | set(actual_effects.updates)
        | set(actual_effects.deletes)
        | {src for src, _dst in actual_effects.renames}
        | {_dst for _src, _dst in actual_effects.renames}
    )
    for analyzer in extensions.semantic_analyzers:
        analyzer.analyze(snapshot, touched)
    for runner in extensions.shadow_workspace_runners:
        runner.verify(snapshot, touched)
