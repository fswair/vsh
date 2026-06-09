#!/usr/bin/env python3
"""Benchmark every vsh command against native shell equivalents."""

from __future__ import annotations as _annotations

import argparse
import os
import shutil
import sys
import tempfile
from datetime import UTC, datetime
from pathlib import Path

PLAYGROUND_DIR = Path(__file__).resolve().parent
sys.path.insert(0, str(PLAYGROUND_DIR))

from benchlib import (  # noqa: E402
    build_cases,
    prepare_workspace,
    print_median_ratios,
    print_table,
    run_case,
    write_json_report,
    write_markdown_report,
    write_plots,
)


def _default_output_dir() -> Path:
    stamp = datetime.now(tz=UTC).strftime("%Y%m%d-%H%M%S")
    return PLAYGROUND_DIR / "reports" / stamp


def _benchmark_max_touched_paths(file_count: int) -> int:
    # Synthetic scale benches should not trip the default interactive safety ceiling.
    return max(500, (file_count * 2) + 50)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--iterations", type=int, default=5, help="repetitions per case/mode")
    parser.add_argument("--file-count", type=int, default=50, help="files created in the workspace")
    parser.add_argument("--file-size", type=int, default=512, help="approximate bytes per file")
    parser.add_argument(
        "--modes",
        default="native,vsh_apply,vsh_full",
        help="comma-separated: native,vsh_apply,vsh_full",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help="directory for JSON/Markdown/plots (default: playground/reports/<timestamp>)",
    )
    parser.add_argument("--no-plots", action="store_true", help="skip plot generation")
    parser.add_argument("--json", action="store_true", help="also print JSON to stdout")
    args = parser.parse_args(argv)

    modes = {part.strip() for part in args.modes.split(",") if part.strip()}
    output_dir = args.output_dir or _default_output_dir()
    cases = build_cases()
    metadata: dict[str, object] = {
        "iterations": args.iterations,
        "file_count": args.file_count,
        "file_size": args.file_size,
        "modes": sorted(modes),
        "commands": len(cases),
        "benchmark_max_touched_paths": _benchmark_max_touched_paths(args.file_count),
    }

    workspace_dir = Path(tempfile.mkdtemp(prefix="vsh-bench-"))
    previous_max_touched = os.environ.get("VSH_MAX_TOUCHED_PATHS")
    try:
        os.environ["VSH_MAX_TOUCHED_PATHS"] = str(_benchmark_max_touched_paths(args.file_count))
        prepare_workspace(workspace_dir, file_count=args.file_count, file_size=args.file_size)
        all_stats = []
        for case in cases:
            all_stats.extend(run_case(workspace_dir, case, iterations=args.iterations, modes=modes))

        output_dir.mkdir(parents=True, exist_ok=True)
        metadata["workspace"] = str(workspace_dir)
        write_json_report(output_dir / "results.json", all_stats, metadata)
        write_markdown_report(output_dir / "report.md", all_stats, cases, metadata)

        plot_paths: list[Path] = []
        if not args.no_plots:
            try:
                plot_paths = write_plots(output_dir / "plots", all_stats)
            except RuntimeError as exc:
                print(f"plot warning: {exc}", file=sys.stderr)

        print(f"report_dir={output_dir}")
        print(
            f"iterations={args.iterations} file_count={args.file_count} "
            f"file_size={args.file_size} commands={len(cases)}"
        )
        print_table(all_stats)
        print_median_ratios(all_stats)
        print()
        print(f"written: {output_dir / 'results.json'}")
        print(f"written: {output_dir / 'report.md'}")
        for plot_path in plot_paths:
            print(f"written: {plot_path}")

        if args.json:
            import json
            from dataclasses import asdict

            print(
                json.dumps(
                    {"metadata": metadata, "results": [asdict(row) for row in all_stats]}, indent=2
                )
            )
    finally:
        if previous_max_touched is None:
            os.environ.pop("VSH_MAX_TOUCHED_PATHS", None)
        else:
            os.environ["VSH_MAX_TOUCHED_PATHS"] = previous_max_touched
        shutil.rmtree(workspace_dir, ignore_errors=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
