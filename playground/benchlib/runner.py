from __future__ import annotations as _annotations

import statistics
import subprocess
from collections.abc import Callable
from pathlib import Path
from typing import Any

from vsh.execute.dispatch import ExecutionContext, apply_command
from vsh.execute.realfs import execute_approved
from vsh.perf.timing import elapsed_ms, perf_counter_ns
from vsh.plans import approve_plan
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace

from .cases import restore_baseline
from .models import BenchmarkCase, BenchmarkStats


def _stdev(values: list[float]) -> float:
    return statistics.stdev(values) if len(values) > 1 else 0.0


def summarize(name: str, mode: str, timings: list[float]) -> BenchmarkStats:
    return BenchmarkStats(
        name=name,
        mode=mode,
        iterations=len(timings),
        median_ms=statistics.median(timings),
        min_ms=min(timings),
        max_ms=max(timings),
        mean_ms=statistics.mean(timings),
        stdev_ms=_stdev(timings),
        samples_ms=tuple(timings),
    )


def time_native(
    workspace: Path,
    shell_cmd: str,
    iterations: int,
    *,
    prepare: Callable[[Path], None] | None = None,
) -> list[float]:
    timings: list[float] = []
    for _ in range(iterations):
        if prepare is not None:
            prepare(workspace)
        start_ns = perf_counter_ns()
        subprocess.run(
            shell_cmd,
            shell=True,
            cwd=workspace,
            check=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        timings.append(elapsed_ms(start_ns))
    return timings


def time_vsh_apply(
    workspace: Path,
    command_builder: Callable[[Path], Any],
    iterations: int,
    *,
    prepare: Callable[[Path], None] | None = None,
) -> list[float]:
    timings: list[float] = []
    for _ in range(iterations):
        if prepare is not None:
            prepare(workspace)
        ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))
        command = command_builder(workspace)
        start_ns = perf_counter_ns()
        effects = apply_command(command, ctx)
        timings.append(effects.execution_time_ms or elapsed_ms(start_ns))
    return timings


def time_vsh_full(
    workspace: Path,
    command_builder: Callable[[Path], Any],
    iterations: int,
    *,
    prepare: Callable[[Path], None] | None = None,
) -> list[float]:
    timings: list[float] = []
    for _ in range(iterations):
        if prepare is not None:
            prepare(workspace)
        command = command_builder(workspace)
        start_ns = perf_counter_ns()
        snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
        result = simulate_command(command, snapshot)
        token = approve_plan(result.plan_id)
        execution = execute_approved(token.token)
        timings.append(execution.total_time_ms or elapsed_ms(start_ns))
        if not execution.applied:
            msg = f"vsh_full failed for {command}: {execution.reason}"
            raise RuntimeError(msg)
    return timings


def run_case(
    workspace: Path,
    case: BenchmarkCase,
    *,
    iterations: int,
    modes: set[str],
) -> list[BenchmarkStats]:
    restore_baseline(workspace)
    stats: list[BenchmarkStats] = []
    if "native" in modes and case.native_shell is not None:
        stats.append(
            summarize(
                case.name,
                "native",
                time_native(workspace, case.native_shell, iterations, prepare=case.prepare),
            )
        )
    if "vsh_apply" in modes:
        stats.append(
            summarize(
                case.name,
                "vsh_apply",
                time_vsh_apply(
                    workspace,
                    case.build_vsh_command,
                    iterations,
                    prepare=case.prepare,
                ),
            )
        )
    if "vsh_full" in modes:
        stats.append(
            summarize(
                case.name,
                "vsh_full",
                time_vsh_full(
                    workspace,
                    case.build_vsh_command,
                    iterations,
                    prepare=case.prepare,
                ),
            )
        )
    return stats
