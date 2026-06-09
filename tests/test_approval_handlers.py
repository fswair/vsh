from __future__ import annotations as _annotations

from pathlib import Path

import pytest
from conftest import with_execution_reason

from vsh.extensions import extensions
from vsh.plans import ApprovalContext, ApprovalDeniedError, ApproveItem, approve_plan
from vsh.plans.approval_handler import run_approval_handlers
from vsh.schemas import LsCommand, TouchCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_run_approval_handlers_noops_when_registry_empty(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(LsCommand(path="."), snapshot)
    from vsh.plans.store import plan_store

    record = plan_store.get(result.plan_id)
    run_approval_handlers(record, auto=False)


def test_approve_plan_invokes_registered_handler(tmp_path: Path) -> None:
    calls: list[tuple[ApprovalContext, ApproveItem]] = []

    def handler(ctx: ApprovalContext, item: ApproveItem) -> None:
        calls.append((ctx, item))

    extensions.approval_handlers.append(handler)
    try:
        workspace = tmp_path / "workspace"
        workspace.mkdir()
        snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
        result = simulate_command(with_execution_reason(TouchCommand(path="new.txt")), snapshot)

        token = approve_plan(result.plan_id)

        assert token.plan_id == result.plan_id
        assert len(calls) == 1
        ctx, item = calls[0]
        assert ctx.auto is False
        assert item.plan_id == result.plan_id
        assert item.shell_preview == result.shell_preview
        assert item.workspace_root == str(workspace.resolve())
        assert item.requires_manual_approval is True
    finally:
        extensions.approval_handlers.clear()


def test_approve_plan_propagates_handler_denial(tmp_path: Path) -> None:
    def handler(ctx: ApprovalContext, item: ApproveItem) -> None:
        _ = ctx
        raise ApprovalDeniedError("blocked by policy", plan_id=item.plan_id)

    extensions.approval_handlers.append(handler)
    try:
        workspace = tmp_path / "workspace"
        workspace.mkdir()
        snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
        result = simulate_command(with_execution_reason(TouchCommand(path="new.txt")), snapshot)

        with pytest.raises(ApprovalDeniedError, match="blocked by policy") as exc_info:
            approve_plan(result.plan_id)

        assert exc_info.value.plan_id == result.plan_id
    finally:
        extensions.approval_handlers.clear()


def test_auto_approve_passes_auto_flag_to_handler(tmp_path: Path) -> None:
    calls: list[bool] = []

    def handler(ctx: ApprovalContext, item: ApproveItem) -> None:
        _ = item
        calls.append(ctx.auto)

    extensions.approval_handlers.append(handler)
    try:
        workspace = tmp_path / "workspace"
        workspace.mkdir()
        snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
        result = simulate_command(LsCommand(path="."), snapshot)
        from vsh.plans import auto_approve_plan

        auto_approve_plan(result.plan_id)

        assert calls == [True]
    finally:
        extensions.approval_handlers.clear()
