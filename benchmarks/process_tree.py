"""Sample aggregate process-tree RSS separately from latency benchmarks (macOS/Linux)."""

from __future__ import annotations

import argparse
import json
import platform
import subprocess
import time
from pathlib import Path


def process_tree_rss(root_pid: int) -> tuple[int, int, int]:
    """Return root RSS, summed descendant RSS, and descendant count in KiB."""
    output = subprocess.run(
        ["ps", "-axo", "pid=,ppid=,rss="], check=True, capture_output=True, text=True
    ).stdout
    processes: dict[int, tuple[int, int]] = {}
    for line in output.splitlines():
        pid, parent, rss = (int(value) for value in line.split())
        processes[pid] = (parent, rss)
    descendants = {root_pid}
    while True:
        discovered = {pid for pid, (parent, _) in processes.items() if parent in descendants}
        if discovered <= descendants:
            break
        descendants.update(discovered)
    root_rss = processes.get(root_pid, (0, 0))[1]
    descendant_rss = sum(processes[pid][1] for pid in descendants if pid in processes)
    return root_rss, descendant_rss, len(descendants & processes.keys())


def measure(command: list[str], interval_ms: int) -> dict[str, object]:
    """Run an explicit benchmark command and sample only its process descendants."""
    samples: list[dict[str, int]] = []
    started = time.perf_counter_ns()
    with subprocess.Popen(command) as process:
        while process.poll() is None:
            root, tree, count = process_tree_rss(process.pid)
            samples.append(
                {
                    "elapsed_ns": time.perf_counter_ns() - started,
                    "root_rss_kib": root,
                    "tree_rss_kib": tree,
                    "process_count": count,
                }
            )
            time.sleep(interval_ms / 1_000)
        returncode = process.wait()
    if returncode:
        raise subprocess.CalledProcessError(returncode, command)
    return {
        "schema": "vsh-process-tree-memory-v1",
        "captured_at_unix_ms": time.time_ns() // 1_000_000,
        "platform": platform.platform(),
        "command": command,
        "interval_ms": interval_ms,
        "elapsed_ns": time.perf_counter_ns() - started,
        "peak_root_rss_kib": max((sample["root_rss_kib"] for sample in samples), default=0),
        "peak_tree_rss_kib": max((sample["tree_rss_kib"] for sample in samples), default=0),
        "max_process_count": max((sample["process_count"] for sample in samples), default=0),
        "samples": samples,
        "limitations": [
            "Sampled RSS sums count shared pages in each process; this is not unique memory or PSS.",
            "Processes and peaks shorter than the sampling interval can be missed.",
            "ps sampling perturbs execution; use a separate uninstrumented run for latency.",
        ],
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--interval-ms", default=50, type=int)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command: list[str] = args.command
    if command and command[0] == "--":
        command = command[1:]
    if not command or args.interval_ms < 1:
        parser.error("provide a command after -- and a positive sampling interval")
    report = measure(command, args.interval_ms)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(args.output)


if __name__ == "__main__":
    main()
