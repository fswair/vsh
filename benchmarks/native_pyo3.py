"""Reproducible native-core/PyO3 boundary benchmark with no benchmark dependency."""

from __future__ import annotations

import argparse
import json
import os
import platform
import sys
import tempfile
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from vsh import RunRequest, Runtime, __version__, engine_kind

PARALLEL_TRANSACTIONS_PER_RUNTIME = 20


@dataclass(frozen=True)
class Sample:
    wall_ns: int
    native_ns: int
    stages_ns: dict[str, int]
    state: str
    changed_paths: int

    @property
    def boundary_ns(self) -> int:
        return max(0, self.wall_ns - self.native_ns)


def _percentile(values: list[int], fraction: float) -> int:
    ordered = sorted(values)
    index = round((len(ordered) - 1) * fraction)
    return ordered[index]


def _distribution(values: list[int]) -> dict[str, int]:
    return {
        "min": min(values),
        "p50": _percentile(values, 0.50),
        "p95": _percentile(values, 0.95),
        "p99": _percentile(values, 0.99),
        "max": max(values),
    }


def _summary(samples: list[Sample]) -> dict[str, Any]:
    walls = [sample.wall_ns for sample in samples]
    boundary = [sample.boundary_ns for sample in samples]
    wall_p50 = _percentile(walls, 0.50)
    boundary_p50 = _percentile(boundary, 0.50)
    stage_names = samples[0].stages_ns
    return {
        "samples": len(samples),
        "wall_ns": _distribution(walls),
        "native_ns": _distribution([sample.native_ns for sample in samples]),
        "boundary_ns": _distribution(boundary),
        "changed_paths": _distribution([sample.changed_paths for sample in samples]),
        "states": {
            state: sum(sample.state == state for sample in samples)
            for state in sorted({sample.state for sample in samples})
        },
        "stages_ns": {
            name: _distribution([sample.stages_ns[name] for sample in samples])
            for name in stage_names
        },
        "boundary_overhead_p50_percent": boundary_p50 / wall_p50 * 100 if wall_p50 else 0.0,
    }


def _sample(runtime: Runtime, request: RunRequest) -> Sample:
    started = time.perf_counter_ns()
    receipt = runtime.preview(request)
    wall_ns = time.perf_counter_ns() - started
    if receipt.state not in {"auto_approved", "pending_approval", "denied"}:
        raise RuntimeError(f"unexpected preview state: {receipt.state}")
    if receipt.state == "auto_approved" and not runtime.discard_preview(receipt.transaction):
        raise RuntimeError("runtime did not retain its auto-approved preview")
    timings = receipt.timings_ns()
    return Sample(
        wall_ns=wall_ns,
        native_ns=timings["total"],
        stages_ns={name: value for name, value in timings.items() if name != "total"},
        state=receipt.state,
        changed_paths=receipt.changed_paths,
    )


def _run_case(
    runtime: Runtime,
    code: str,
    name: str,
    iterations: int,
    expected_state: str,
) -> dict[str, Any]:
    warmup = _sample(runtime, RunRequest(code, intent=f"{name}-warmup"))
    if warmup.state != expected_state:
        raise RuntimeError(f"{name} warmup returned {warmup.state!r}, expected {expected_state!r}")
    samples = [
        _sample(runtime, RunRequest(code, intent=f"{name}-{index}")) for index in range(iterations)
    ]
    if unexpected := next(
        (sample.state for sample in samples if sample.state != expected_state), None
    ):
        raise RuntimeError(f"{name} returned {unexpected!r}, expected {expected_state!r}")
    return _summary(samples)


def _small_fixture(root: Path) -> None:
    for index in range(20):
        (root / f"input-{index:02}.txt").write_bytes((f"line-{index}\n" * 32).encode())


def _large_tree_fixture(root: Path) -> None:
    large_tree = root / "large-tree"
    large_tree.mkdir()
    for directory_index in range(100):
        directory = large_tree / f"dir-{directory_index:03}"
        directory.mkdir()
        for file_index in range(100):
            (directory / f"file-{file_index:03}.txt").touch()


def _read_ten_program() -> str:
    reads = [f"Path('/workspace/input-{index:02}.txt').read_bytes()" for index in range(10)]
    return "from pathlib import Path\nlen(b''.join([\n    " + ",\n    ".join(reads) + "\n]))"


def _edit_twenty_program() -> str:
    writes = [
        f"Path('/workspace/output-{index:02}.txt').write_text('value-{index}')"
        for index in range(20)
    ]
    return "from pathlib import Path\n" + "\n".join(writes) + "\n20"


def _search_ten_thousand_program() -> str:
    return """import os
matches = 0
root = '/workspace/large-tree'
for directory in os.listdir(root):
    for name in os.listdir(root + '/' + directory):
        if name.endswith('7.txt'):
            matches += 1
matches
"""


def _glob_ten_thousand_with_vsh_program() -> str:
    return "len(vsh_glob('**/*7.txt', path='/workspace/large-tree', max_results=1000))\n"


def _rename_subtree_program() -> str:
    return "import os\nos.rename('/workspace/large-tree/dir-000', '/workspace/moved-tree')\nNone\n"


def _delete_subtree_program() -> str:
    return """import os
root = '/workspace/large-tree/dir-000'
for name in os.listdir(root):
    os.unlink(root + '/' + name)
os.rmdir(root)
None
"""


def _delete_subtree_with_vsh_program() -> str:
    return "vsh_remove('/workspace/large-tree/dir-000', recursive=True)\nNone\n"


def _massive_delete_program() -> str:
    return """import os
root = '/workspace/large-tree'
for directory in sorted(os.listdir(root))[:50]:
    subtree = root + '/' + directory
    for name in os.listdir(subtree):
        os.unlink(subtree + '/' + name)
    os.rmdir(subtree)
None
"""


def _repeated_cold_case(iterations: int) -> dict[str, Any]:
    opens: list[int] = []
    first_calls: list[int] = []
    for index in range(iterations):
        with tempfile.TemporaryDirectory(prefix="vsh-cold-") as raw_root:
            open_started = time.perf_counter_ns()
            runtime = Runtime.open(raw_root)
            opens.append(time.perf_counter_ns() - open_started)
            sample = _sample(runtime, RunRequest("None", intent=f"cold-{index}"))
            if sample.state != "auto_approved":
                raise RuntimeError(f"cold first call returned {sample.state!r}")
            first_calls.append(sample.wall_ns)
    return {
        "samples": iterations,
        "runtime_open_ns": _distribution(opens),
        "first_call_wall_ns": _distribution(first_calls),
    }


def _parallel_case(workers: int) -> dict[str, Any]:
    directories = [tempfile.TemporaryDirectory(prefix="vsh-parallel-") for _ in range(workers)]
    try:
        runtimes = [Runtime.open(directory.name) for directory in directories]
        for index, runtime in enumerate(runtimes):
            _sample(runtime, RunRequest("None", intent=f"parallel-warmup-{index}"))

        sequential_started = time.perf_counter_ns()
        for index, runtime in enumerate(runtimes):
            for transaction in range(PARALLEL_TRANSACTIONS_PER_RUNTIME):
                _sample(
                    runtime,
                    RunRequest(
                        "sum(range(10000))",
                        intent=f"sequential-{index}-{transaction}",
                    ),
                )
        sequential_ns = time.perf_counter_ns() - sequential_started

        def execute(item: tuple[int, Runtime]) -> list[Sample]:
            index, runtime = item
            return [
                _sample(
                    runtime,
                    RunRequest(
                        "sum(range(10000))",
                        intent=f"parallel-{index}-{transaction}",
                    ),
                )
                for transaction in range(PARALLEL_TRANSACTIONS_PER_RUNTIME)
            ]

        parallel_started = time.perf_counter_ns()
        with ThreadPoolExecutor(max_workers=workers) as executor:
            sample_groups = list(executor.map(execute, enumerate(runtimes)))
        parallel_ns = time.perf_counter_ns() - parallel_started
        samples = [sample for group in sample_groups for sample in group]
        return {
            "workers": workers,
            "transactions_per_runtime": PARALLEL_TRANSACTIONS_PER_RUNTIME,
            "total_transactions": workers * PARALLEL_TRANSACTIONS_PER_RUNTIME,
            "sequential_wall_ns": sequential_ns,
            "parallel_wall_ns": parallel_ns,
            "speedup": sequential_ns / parallel_ns if parallel_ns else 0.0,
            "sample_native_ns": [sample.native_ns for sample in samples],
        }
    finally:
        for directory in directories:
            directory.cleanup()


def run(iterations: int, cold_iterations: int, parallel_workers: int) -> dict[str, Any]:
    if iterations < 3:
        raise ValueError("iterations must be at least 3")
    if parallel_workers < 2:
        raise ValueError("parallel workers must be at least 2")
    if cold_iterations < 3:
        raise ValueError("cold iterations must be at least 3")

    with tempfile.TemporaryDirectory(prefix="vsh-benchmark-") as raw_root:
        root = Path(raw_root)
        _small_fixture(root)

        open_started = time.perf_counter_ns()
        runtime = Runtime.open(root)
        open_ns = time.perf_counter_ns() - open_started
        cold = _sample(runtime, RunRequest("None", intent="cold-first-worker"))

        cases = {
            "noop": _run_case(runtime, "None", "noop", iterations, "auto_approved"),
            "read_10": _run_case(
                runtime, _read_ten_program(), "read-10", iterations, "auto_approved"
            ),
            "edit_20": _run_case(
                runtime, _edit_twenty_program(), "edit-20", iterations, "auto_approved"
            ),
        }

    with tempfile.TemporaryDirectory(prefix="vsh-benchmark-large-") as raw_large_root:
        large_root = Path(raw_large_root)
        fixture_started = time.perf_counter_ns()
        _large_tree_fixture(large_root)
        large_fixture_ns = time.perf_counter_ns() - fixture_started
        large_open_started = time.perf_counter_ns()
        large_runtime = Runtime.open(large_root)
        large_runtime_open_ns = time.perf_counter_ns() - large_open_started
        cases["search_10k"] = _run_case(
            large_runtime,
            _search_ten_thousand_program(),
            "search-10k",
            iterations,
            "auto_approved",
        )
        cases["vsh_glob_10k"] = _run_case(
            large_runtime,
            _glob_ten_thousand_with_vsh_program(),
            "vsh-glob-10k",
            iterations,
            "auto_approved",
        )
        cases["rename_subtree_100"] = _run_case(
            large_runtime,
            _rename_subtree_program(),
            "rename-subtree-100",
            iterations,
            "pending_approval",
        )
        cases["delete_subtree_100"] = _run_case(
            large_runtime,
            _delete_subtree_program(),
            "delete-subtree-100",
            iterations,
            "pending_approval",
        )
        cases["vsh_remove_subtree_100"] = _run_case(
            large_runtime,
            _delete_subtree_with_vsh_program(),
            "vsh-remove-subtree-100",
            iterations,
            "pending_approval",
        )
        cases["massive_delete_5k"] = _run_case(
            large_runtime,
            _massive_delete_program(),
            "massive-delete-5k",
            iterations,
            "pending_approval",
        )

    return {
        "schema": "vsh-native-pyo3-benchmark-v2",
        "captured_at_unix_ms": time.time_ns() // 1_000_000,
        "environment": {
            "vsh_version": __version__,
            "engine": engine_kind(),
            "python": sys.version,
            "platform": platform.platform(),
            "machine": platform.machine(),
            "cpu_count": os.cpu_count(),
            "worker": os.environ.get("VSH_MONTY_WORKER", "wheel/default PATH resolution"),
        },
        "iterations": iterations,
        "cold": {
            "runtime_open_ns": open_ns,
            "first_call_wall_ns": cold.wall_ns,
            "first_call_native_ns": cold.native_ns,
            "first_call_boundary_ns": cold.boundary_ns,
            "repeated": _repeated_cold_case(cold_iterations),
        },
        "large_tree": {
            "directories": 100,
            "files": 10_000,
            "fixture_ns": large_fixture_ns,
            "runtime_open_ns": large_runtime_open_ns,
        },
        "cases": cases,
        "parallel_independent_runtimes": _parallel_case(parallel_workers),
    }


def _markdown(report: dict[str, Any]) -> str:
    rows = []
    cases = report["cases"]
    assert isinstance(cases, dict)
    for name, raw_case in cases.items():
        assert isinstance(raw_case, dict)
        wall = raw_case["wall_ns"]
        boundary = raw_case["boundary_ns"]
        stages = raw_case["stages_ns"]
        states = raw_case["states"]
        changed_paths = raw_case["changed_paths"]
        assert isinstance(wall, dict)
        assert isinstance(boundary, dict)
        assert isinstance(stages, dict)
        assert isinstance(states, dict)
        assert isinstance(changed_paths, dict)
        state = ", ".join(states)
        rows.append(
            f"| {name} | {state} | {changed_paths['p50']} | "
            f"{wall['p50'] / 1_000_000:.3f} | {wall['p99'] / 1_000_000:.3f} | "
            f"{stages['snapshot']['p50'] / 1_000_000:.3f} | "
            f"{stages['execute']['p50'] / 1_000_000:.3f} | "
            f"{boundary['p50'] / 1_000:.1f} | {raw_case['boundary_overhead_p50_percent']:.2f}% |"
        )
    parallel = report["parallel_independent_runtimes"]
    assert isinstance(parallel, dict)
    cold = report["cold"]
    assert isinstance(cold, dict)
    repeated_cold = cold["repeated"]
    assert isinstance(repeated_cold, dict)
    cold_open = repeated_cold["runtime_open_ns"]
    cold_first = repeated_cold["first_call_wall_ns"]
    assert isinstance(cold_open, dict)
    assert isinstance(cold_first, dict)
    return "\n".join(
        [
            "# VSH native core / PyO3 benchmark",
            "",
            "| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | boundary p50 µs | boundary p50 % |",
            "|---|---|---:|---:|---:|---:|---:|---:|---:|",
            *rows,
            "",
            f"Repeated cold runtime-open p50/p99: {cold_open['p50'] / 1_000_000:.3f}/"
            f"{cold_open['p99'] / 1_000_000:.3f} ms; first-call p50/p99: "
            f"{cold_first['p50'] / 1_000_000:.3f}/{cold_first['p99'] / 1_000_000:.3f} ms.",
            "",
            f"Independent-runtime parallel speedup: {parallel['speedup']:.2f}x "
            f"across {parallel['workers']} workers.",
            "",
        ]
    )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--iterations", type=int, default=30)
    parser.add_argument("--cold-iterations", type=int, default=30)
    parser.add_argument("--parallel-workers", type=int, default=4)
    parser.add_argument(
        "--output", type=Path, default=Path("benchmarks/results/local/native-pyo3.json")
    )
    arguments = parser.parse_args()

    report = run(arguments.iterations, arguments.cold_iterations, arguments.parallel_workers)
    arguments.output.parent.mkdir(parents=True, exist_ok=True)
    arguments.output.write_text(
        json.dumps(report, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    markdown = arguments.output.with_suffix(".md")
    markdown.write_text(_markdown(report), encoding="utf-8")
    print(markdown)


if __name__ == "__main__":
    main()
