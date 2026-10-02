"""Repeatable public-Python Bash measurements on a disposable fixture (no live models)."""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import statistics
import time
from pathlib import Path
from tempfile import TemporaryDirectory
from typing import TypedDict

from vsh import BashConfig, Language, Runtime


class PreviewSample(TypedDict):
    wall: int
    state: str
    changed_paths: int
    os_calls: int
    read_bytes: int
    write_bytes: int
    stages: dict[str, int]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--iterations", type=int, default=50)
    parser.add_argument("--cold-iterations", type=int, default=20)
    args = parser.parse_args()
    if args.iterations < 20:
        parser.error("at least 20 retained samples are required")
    if args.cold_iterations < 20:
        parser.error("at least 20 retained cold samples are required")
    worker = args.worker.resolve(strict=True)
    report: dict[str, object] = {
        "schema": "vsh-public-bash-benchmark-v1",
        "captured_at_unix_ms": time.time_ns() // 1_000_000,
        "platform": platform.platform(),
        "python": platform.python_version(),
        "worker_sha256": hashlib.sha256(worker.read_bytes()).hexdigest(),
        "iterations": args.iterations,
        "cold_iterations": args.cold_iterations,
        "units": "nanoseconds",
        "notes": [
            "Release-profile native extension and worker; previews, not commit latency.",
            "Each call takes a fresh snapshot; warm reuse retains the worker process, not interpreter state.",
            "Cold includes Runtime.open plus first Bash preview on the same fixture.",
            "RSS must be sampled in a separate process_tree.py run, not in this latency run.",
        ],
    }
    cases: dict[str, object] = {}
    cold: list[int] = []
    with TemporaryDirectory(prefix="vsh-bash-benchmark-") as temporary:
        workspace = Path(temporary)
        for index in range(20):
            (workspace / f"input-{index:02}.txt").write_text("line\n" * 32)
        (workspace / "large.bin").write_bytes(b"\xff\x00" * (512 * 1024))
        for _ in range(args.cold_iterations):
            started = time.perf_counter_ns()
            runtime = Runtime.open(workspace, bash=BashConfig(worker_path=worker))
            receipt = runtime.preview(":", language=Language.BASH)
            cold.append(time.perf_counter_ns() - started)
            assert runtime.discard_preview(receipt.transaction)
            del runtime
        runtime = Runtime.open(workspace, bash=BashConfig(worker_path=worker))
        for name, source in {
            "noop": ":",
            "read_10": "; ".join(f"cat input-{index:02}.txt" for index in range(10)),
            "pipeline_1m": "cat large.bin | wc -c",
            "edit_20": "; ".join(f"printf new > input-{index:02}.txt" for index in range(20)),
            "copy_1m": "cp large.bin copied.bin",
            "chmod_20": "; ".join(f"chmod 600 input-{index:02}.txt" for index in range(20)),
        }.items():
            samples: list[PreviewSample] = []
            for index in range(args.iterations + 1):
                started = time.perf_counter_ns()
                receipt = runtime.preview(source, language=Language.BASH, intent=f"{name}-{index}")
                wall = time.perf_counter_ns() - started
                if index:
                    samples.append(
                        {
                            "wall": wall,
                            "state": receipt.state,
                            "changed_paths": receipt.changed_paths,
                            "os_calls": receipt.os_calls,
                            "read_bytes": receipt.read_bytes,
                            "write_bytes": receipt.write_bytes,
                            "stages": dict(receipt.timings_ns()),
                        }
                    )
                if receipt.state == "auto_approved":
                    assert runtime.discard_preview(receipt.transaction)
            ordered = sorted(sample["wall"] for sample in samples)
            cases[name] = {
                "p50": statistics.median(ordered),
                "p95": ordered[(len(ordered) - 1) * 95 // 100],
                "samples": samples,
            }
    report["cases"] = cases
    report["cold"] = {
        "p50": statistics.median(cold),
        "p95": sorted(cold)[(len(cold) - 1) * 95 // 100],
        "samples": cold,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(args.output)


if __name__ == "__main__":
    main()
