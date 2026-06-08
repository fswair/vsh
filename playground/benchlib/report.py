from __future__ import annotations as _annotations

import json
from dataclasses import asdict
from pathlib import Path

from .models import BenchmarkCase, BenchmarkStats


def print_table(rows: list[BenchmarkStats]) -> None:
    header = (
        f"{'command':<12} {'mode':<12} {'median_ms':>11} {'min_ms':>9} "
        f"{'max_ms':>9} {'mean_ms':>10} {'n':>4}"
    )
    print(header)
    print("-" * len(header))
    for row in rows:
        print(
            f"{row.name:<12} {row.mode:<12} {row.median_ms:11.3f} {row.min_ms:9.3f} "
            f"{row.max_ms:9.3f} {row.mean_ms:10.3f} {row.iterations:4d}"
        )


def print_median_ratios(rows: list[BenchmarkStats]) -> None:
    by_name: dict[str, dict[str, float]] = {}
    for row in rows:
        by_name.setdefault(row.name, {})[row.mode] = row.median_ms
    print()
    print("median_ms ratios (vsh / native, >1 means vsh slower):")
    for name, modes in sorted(by_name.items()):
        native = modes.get("native")
        if native is None or native <= 0:
            continue
        apply_ratio = modes.get("vsh_apply", 0.0) / native
        full_ratio = modes.get("vsh_full", 0.0) / native
        print(
            f"  {name:<12} apply={apply_ratio:6.2f}x  full={full_ratio:6.2f}x  "
            f"(native={native:.3f} apply={modes.get('vsh_apply', 0):.3f} "
            f"full={modes.get('vsh_full', 0):.3f})"
        )


def write_json_report(path: Path, rows: list[BenchmarkStats], metadata: dict[str, object]) -> None:
    payload = {
        "metadata": metadata,
        "results": [asdict(row) for row in rows],
    }
    path.write_text(json.dumps(payload, indent=2), encoding="utf-8")


def write_markdown_report(
    path: Path,
    rows: list[BenchmarkStats],
    cases: list[BenchmarkCase],
    metadata: dict[str, object],
) -> None:
    skipped = [case.name for case in cases if case.native_shell is None]
    lines = [
        "# vsh benchmark report",
        "",
        "## Run metadata",
        "",
    ]
    for key, value in metadata.items():
        lines.append(f"- **{key}**: `{value}`")
    if skipped:
        lines.extend(["", f"- **native_skipped**: `{', '.join(skipped)}`"])
    lines.extend(
        [
            "",
            "## Median latency (ms)",
            "",
            "| command | mode | median | min | max | mean | n |",
            "|---------|------|-------:|----:|----:|-----:|--:|",
        ]
    )
    for row in rows:
        lines.append(
            f"| {row.name} | {row.mode} | {row.median_ms:.3f} | {row.min_ms:.3f} | "
            f"{row.max_ms:.3f} | {row.mean_ms:.3f} | {row.iterations} |"
        )

    by_name: dict[str, dict[str, BenchmarkStats]] = {}
    for row in rows:
        by_name.setdefault(row.name, {})[row.mode] = row

    lines.extend(
        [
            "",
            "## Median ratios vs native",
            "",
            "| command | vsh_apply | vsh_full |",
            "|---------|----------:|---------:|",
        ]
    )
    for name in sorted(by_name):
        native = by_name[name].get("native")
        if native is None or native.median_ms <= 0:
            continue
        apply_row = by_name[name].get("vsh_apply")
        full_row = by_name[name].get("vsh_full")
        apply_ratio = (apply_row.median_ms / native.median_ms) if apply_row else 0.0
        full_ratio = (full_row.median_ms / native.median_ms) if full_row else 0.0
        lines.append(f"| {name} | {apply_ratio:.2f}x | {full_ratio:.2f}x |")

    notes = [case for case in cases if case.native_note]
    if notes:
        lines.extend(["", "## Notes", ""])
        for case in notes:
            lines.append(f"- **{case.name}**: {case.native_note}")

    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
