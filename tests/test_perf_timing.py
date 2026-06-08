from __future__ import annotations as _annotations

from pathlib import Path

import pytest

from vsh.effects import ActualEffects
from vsh.execute.dispatch import ExecutionContext, apply_command
from vsh.execute.realfs import execute_approved
from vsh.perf.timing import elapsed_ms, perf_counter_ns, stamp_execution_time
from vsh.plans import approve_plan
from vsh.schemas import CatCommand, LsCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_elapsed_ms_reports_positive_interval() -> None:
    start_ns = perf_counter_ns()
    elapsed = elapsed_ms(start_ns)
    assert elapsed >= 0.0


def test_stamp_execution_time_attaches_field() -> None:
    effects = ActualEffects(reads=["/tmp"])
    stamped = stamp_execution_time(effects, perf_counter_ns())
    assert isinstance(stamped, ActualEffects)
    assert stamped.execution_time_ms is not None
    assert stamped.execution_time_ms >= 0.0


def test_stamp_execution_time_rejects_non_effects() -> None:
    with pytest.raises(TypeError, match="expected ActualEffects"):
        stamp_execution_time("not-effects", perf_counter_ns())


def test_apply_command_records_execution_time_ms(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "notes.txt"
    target.write_text("hello\n", encoding="utf-8")
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))

    effects = apply_command(CatCommand(path="notes.txt"), ctx)

    assert effects.execution_time_ms is not None
    assert effects.execution_time_ms >= 0.0


def test_simulate_command_records_simulation_time_ms(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "alpha.txt").write_text("x\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    result = simulate_command(LsCommand(path="."), snapshot)

    assert result.simulation_time_ms is not None
    assert result.simulation_time_ms >= 0.0


def test_execute_approved_records_pipeline_timings(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "alpha.txt").write_text("x\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    simulation = simulate_command(LsCommand(path="."), snapshot)
    token = approve_plan(simulation.plan_id)

    execution = execute_approved(token.token)

    assert execution.applied is True
    assert execution.total_time_ms is not None
    assert execution.revalidation_time_ms is not None
    assert execution.apply_time_ms is not None
    assert execution.total_time_ms >= execution.apply_time_ms
    assert execution.actual_effects is not None
    assert execution.actual_effects.execution_time_ms is not None
