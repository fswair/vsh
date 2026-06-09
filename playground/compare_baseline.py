#!/usr/bin/env python3
"""Compare a benchmark run directory against a frozen baseline."""

from __future__ import annotations as _annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

REGRESSION_RATIO_THRESHOLD = 0.10


def _load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def _median_ratios(results: list[dict[str, Any]]) -> dict[str, dict[str, float]]:
    by_name: dict[str, dict[str, float]] = {}
    for row in results:
        by_name.setdefault(row["name"], {})[row["mode"]] = float(row["median_ms"])
    ratios: dict[str, dict[str, float]] = {}
    for name, modes in by_name.items():
        native = modes.get("native")
        if native is None or native <= 0:
            continue
        ratios[name] = {
            mode: modes[mode] / native for mode in ("vsh_apply", "vsh_full") if mode in modes
        }
    return ratios


def _compare_playground(baseline_dir: Path, current_dir: Path) -> tuple[str, bool]:
    baseline_path = baseline_dir / "playground" / "results.json"
    current_path = current_dir / "playground" / "results.json"
    if not baseline_path.is_file():
        return f"missing baseline playground results: {baseline_path}", False
    if not current_path.is_file():
        return f"missing current playground results: {current_path}", False

    baseline = _load_json(baseline_path)
    current = _load_json(current_path)
    base_ratios = _median_ratios(baseline["results"])
    curr_ratios = _median_ratios(current["results"])

    lines = ["## Playground ratio diff (current / baseline)", ""]
    lines.append("| command | mode | baseline | current | delta | regressed |")
    lines.append("|---------|------|---------:|--------:|------:|:---------:|")
    regressed = False
    for name in sorted(set(base_ratios) | set(curr_ratios)):
        for mode in ("vsh_apply", "vsh_full"):
            base = base_ratios.get(name, {}).get(mode)
            curr = curr_ratios.get(name, {}).get(mode)
            if base is None or curr is None:
                continue
            delta = curr - base
            is_regressed = delta > REGRESSION_RATIO_THRESHOLD
            regressed = regressed or is_regressed
            lines.append(
                f"| {name} | {mode} | {base:.3f}x | {curr:.3f}x | {delta:+.3f}x | {is_regressed} |"
            )
    return "\n".join(lines), not regressed


def _agent_metrics(payload: dict[str, Any]) -> dict[str, Any]:
    comparison = payload.get("comparison", payload)
    if "vsh" in comparison:
        vsh = comparison["vsh"]
        native = comparison["native"]
        return {
            "vsh_duration_ms": vsh["duration_ms"],
            "native_duration_ms": native["duration_ms"],
            "vsh_input_tokens": vsh["usage"]["input_tokens"],
            "native_input_tokens": native["usage"]["input_tokens"],
            "vsh_tool_calls": vsh["usage"].get("tool_calls", len(vsh.get("tool_names", []))),
            "native_tool_calls": native["usage"].get(
                "tool_calls", len(native.get("tool_names", []))
            ),
            "vsh_validation_passed": vsh["validation_passed"],
            "native_validation_passed": native["validation_passed"],
        }
    summary = payload.get("summary", {})
    return {
        "vsh_duration_ms": summary.get("vsh_duration_ms_median"),
        "native_duration_ms": summary.get("native_duration_ms_median"),
        "vsh_input_tokens": summary.get("vsh_input_tokens_median"),
        "native_input_tokens": summary.get("native_input_tokens_median"),
        "vsh_tool_calls": summary.get("vsh_tool_calls_median"),
        "native_tool_calls": summary.get("native_tool_calls_median"),
        "vsh_validation_passed": summary.get("vsh_validation_passed_all", True),
        "native_validation_passed": summary.get("native_validation_passed_all", True),
    }


def _agent_result_path(root: Path) -> Path | None:
    for candidate in (
        root / "agent-context" / "multi_run_summary.json",
        root / "agent-context" / "comparison.json",
        root / "multi_run_summary.json",
        root / "comparison.json",
    ):
        if candidate.is_file():
            return candidate
    return None


def _compare_agent(baseline_dir: Path, current_dir: Path) -> tuple[str, bool]:
    baseline_path = _agent_result_path(baseline_dir)
    current_path = _agent_result_path(current_dir)
    if baseline_path is None:
        return f"missing baseline agent results under: {baseline_dir}", False
    if current_path is None:
        return f"missing current agent results under: {current_dir}", False

    base = _agent_metrics(_load_json(baseline_path))
    curr = _agent_metrics(_load_json(current_path))
    lines = ["## Agent context diff", ""]
    lines.append("| metric | baseline | current | delta |")
    lines.append("|--------|---------:|--------:|------:|")
    passed = True
    for key in sorted(base):
        b = base[key]
        c = curr.get(key)
        if isinstance(b, (int, float)) and isinstance(c, (int, float)):
            lines.append(f"| {key} | {b} | {c} | {c - b:+.1f} |")
            if key == "vsh_input_tokens" and c > b * 1.1:
                passed = False
            if key == "vsh_tool_calls" and c > b:
                passed = False
        else:
            lines.append(f"| {key} | {b} | {c} | — |")
            if key.endswith("validation_passed") and c is False:
                passed = False
    return "\n".join(lines), passed


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--current", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=None)
    args = parser.parse_args(argv)

    playground_md, playground_ok = _compare_playground(args.baseline, args.current)
    agent_md, agent_ok = _compare_agent(args.baseline, args.current)
    overall_ok = playground_ok and agent_ok

    report = "\n".join(
        [
            "# Baseline comparison",
            "",
            f"- baseline: `{args.baseline}`",
            f"- current: `{args.current}`",
            f"- overall: **{'PASS' if overall_ok else 'FAIL'}**",
            "",
            playground_md,
            "",
            agent_md,
        ]
    )
    if args.output is not None:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(report, encoding="utf-8")
        print(f"written: {args.output}")
    print(report)
    return 0 if overall_ok else 1


if __name__ == "__main__":
    sys.exit(main())
