"""Derive optimization evidence from retained, same-protocol release benchmark reports."""

from __future__ import annotations

import argparse
import json
import statistics
from pathlib import Path
from typing import Any


def read(directory: Path, name: str) -> dict[str, Any]:
    value = json.loads((directory / f"{name}.json").read_text(encoding="utf-8"))
    if not isinstance(value, dict):
        raise ValueError(f"{name} is not a report object")
    return value


def compare(directory: Path) -> dict[str, Any]:
    surfaces: dict[str, Any] = {}
    for surface, baseline_name in [
        ("rust", "baseline-rust"),
        ("python", "baseline-release-python"),
    ]:
        baseline = read(directory, baseline_name)
        final = read(directory, f"final-{surface}")
        confirmation = read(directory, f"confirmation-{surface}")
        if (
            baseline["iterations"] != final["iterations"]
            or final["iterations"] != confirmation["iterations"]
        ):
            raise ValueError("Cannot compare different sampling protocols")
        if baseline["cases"].keys() != final["cases"].keys():
            raise ValueError("Cannot compare different workload matrices")
        cases = {}
        for name, before in baseline["cases"].items():
            after, repeat = final["cases"][name], confirmation["cases"][name]
            for report in (after, repeat):
                for field in (("state" if surface == "rust" else "states"), "changed_paths"):
                    if before[field] != report[field]:
                        raise ValueError(f"{surface}/{name} changed its {field} contract")
            cases[name] = {
                "baseline_p50_ms": before["wall_ns"]["p50"] / 1e6,
                "final_p50_ms": after["wall_ns"]["p50"] / 1e6,
                "baseline_p95_ms": before["wall_ns"]["p95"] / 1e6,
                "final_p95_ms": after["wall_ns"]["p95"] / 1e6,
                "confirmation_p50_ms": repeat["wall_ns"]["p50"] / 1e6,
                "confirmation_p95_ms": repeat["wall_ns"]["p95"] / 1e6,
                "p50_reduction_pct": 100 * (1 - after["wall_ns"]["p50"] / before["wall_ns"]["p50"]),
                "baseline_stages_ns": before["stages_ns"],
                "final_stages_ns": after["stages_ns"],
            }
        before_memory = read(directory, f"baseline-{surface}-memory")
        after_memory = read(directory, f"final-{surface}-memory")
        surfaces[surface] = {
            "baseline": baseline_name,
            "samples_per_case": final["iterations"],
            "cases": cases,
            "memory": {
                phase: {
                    key: report[key]
                    for key in (
                        "peak_root_rss_kib",
                        "peak_tree_rss_kib",
                        "max_process_count",
                        "interval_ms",
                    )
                }
                for phase, report in [("baseline", before_memory), ("final", after_memory)]
            },
        }
    before_policy = read(directory, "baseline-policy-path")
    after_policy = read(directory, "final-policy-path")
    micro = {}
    for key in ("authorize_ns", "parse_ns"):
        before = statistics.median(before_policy[key])
        after = statistics.median(after_policy[key])
        micro[key] = {
            "baseline_median_ns": before,
            "final_median_ns": after,
            "reduction_pct": 100 * (1 - after / before),
        }
    return {
        "schema": "vsh-optimization-comparison-v1",
        "surfaces": surfaces,
        "microbenchmark_10000_paths": micro,
        "limitations": [
            "Local sequential release runs, not randomized trials or a universal performance guarantee.",
            "Confirmation runs are separate observations, not selected replacements for the final run.",
            "Small calls and durable I/O show run-to-run variation; report regressions as well as gains.",
            "Summed sampled RSS double-counts shared pages and can miss short peaks; it is not PSS.",
            "No model calls or billing were measured; resource savings are not monetary savings.",
            "The initial baseline-python report used a debug extension and is excluded.",
        ],
    }


def markdown(report: dict[str, Any]) -> str:
    lines = ["# Optimization comparison", ""]
    for surface, data in report["surfaces"].items():
        lines.extend(
            [
                f"## {surface.title()} release",
                "",
                "| Case | Before p50 ms | Final p50 ms | Change | Final p95 ms | Repeat p50 ms |",
                "|---|---:|---:|---:|---:|---:|",
            ]
        )
        for name, case in data["cases"].items():
            lines.append(
                f"| {name} | {case['baseline_p50_ms']:.3f} | {case['final_p50_ms']:.3f} | "
                f"{case['p50_reduction_pct']:.1f}% lower | {case['final_p95_ms']:.3f} | "
                f"{case['confirmation_p50_ms']:.3f} |"
            )
        lines.extend(
            [
                "",
                "Sampled RSS (separate instrumented run):",
                "",
                "| Phase | Root MiB | Summed tree MiB | Max processes |",
                "|---|---:|---:|---:|",
            ]
        )
        for phase, memory in data["memory"].items():
            lines.append(
                f"| {phase} | {memory['peak_root_rss_kib'] / 1024:.2f} | "
                f"{memory['peak_tree_rss_kib'] / 1024:.2f} | {memory['max_process_count']} |"
            )
        lines.append("")
    lines.extend(["## Limitations", "", *[f"- {item}" for item in report["limitations"]], ""])
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    report = compare(args.results)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    args.output.with_suffix(".md").write_text(markdown(report), encoding="utf-8")
    print(args.output)


if __name__ == "__main__":
    main()
