from __future__ import annotations as _annotations

from pathlib import Path

from .models import BenchmarkStats

MODE_COLORS = {
    "native": "#4C72B0",
    "vsh_apply": "#55A868",
    "vsh_full": "#C44E52",
}


def write_plots(output_dir: Path, rows: list[BenchmarkStats]) -> list[Path]:
    try:
        import matplotlib.pyplot as plt
    except ImportError as exc:
        msg = "matplotlib is required for plots: uv sync --group dev"
        raise RuntimeError(msg) from exc

    output_dir.mkdir(parents=True, exist_ok=True)
    written: list[Path] = []

    commands = sorted({row.name for row in rows})
    modes = ["native", "vsh_apply", "vsh_full"]
    by_key = {(row.name, row.mode): row for row in rows}

    plt.style.use("seaborn-v0_8-whitegrid")

    # 1) Median latency grouped bars
    fig, ax = plt.subplots(figsize=(14, max(6, len(commands) * 0.35)))
    y_positions = list(range(len(commands)))
    bar_height = 0.24
    for mode_index, mode in enumerate(modes):
        medians = [by_key.get((cmd, mode)) for cmd in commands]
        values = [item.median_ms if item else 0.0 for item in medians]
        offsets = [y + (mode_index - 1) * bar_height for y in y_positions]
        bars = ax.barh(
            offsets,
            values,
            height=bar_height,
            label=mode,
            color=MODE_COLORS[mode],
            alpha=0.9,
        )
        for bar, item in zip(bars, medians, strict=True):
            if item is None:
                continue
            ax.text(
                bar.get_width() + 0.05,
                bar.get_y() + bar.get_height() / 2,
                f"{item.median_ms:.2f}",
                va="center",
                fontsize=7,
            )
    ax.set_yticks(y_positions)
    ax.set_yticklabels(commands)
    ax.set_xlabel("median latency (ms)")
    ax.set_title("vsh vs native — median latency per command")
    ax.legend(loc="lower right")
    fig.tight_layout()
    median_path = output_dir / "median_latency.png"
    fig.savefig(median_path, dpi=160)
    plt.close(fig)
    written.append(median_path)

    # 2) Min/median/max range per mode (error bars)
    for mode in modes:
        mode_rows = [by_key.get((cmd, mode)) for cmd in commands]
        if not any(mode_rows):
            continue
        fig, ax = plt.subplots(figsize=(14, max(6, len(commands) * 0.35)))
        y_pos = list(range(len(commands)))
        medians: list[float] = []
        yerr_lower: list[float] = []
        yerr_upper: list[float] = []
        for item in mode_rows:
            if item is None:
                medians.append(0.0)
                yerr_lower.append(0.0)
                yerr_upper.append(0.0)
                continue
            medians.append(item.median_ms)
            yerr_lower.append(max(0.0, item.median_ms - item.min_ms))
            yerr_upper.append(max(0.0, item.max_ms - item.median_ms))
        ax.barh(
            y_pos,
            medians,
            xerr=[yerr_lower, yerr_upper],
            color=MODE_COLORS[mode],
            alpha=0.85,
            capsize=3,
        )
        ax.set_yticks(y_pos)
        ax.set_yticklabels(commands)
        ax.set_xlabel("latency (ms)")
        ax.set_title(f"{mode}: median with min/max range")
        fig.tight_layout()
        range_path = output_dir / f"range_{mode}.png"
        fig.savefig(range_path, dpi=160)
        plt.close(fig)
        written.append(range_path)

    # 3) Ratio vs native (median)
    ratio_commands: list[str] = []
    apply_ratios: list[float] = []
    full_ratios: list[float] = []
    for cmd in commands:
        native = by_key.get((cmd, "native"))
        apply_row = by_key.get((cmd, "vsh_apply"))
        full_row = by_key.get((cmd, "vsh_full"))
        if native is None or native.median_ms <= 0:
            continue
        ratio_commands.append(cmd)
        apply_ratios.append((apply_row.median_ms / native.median_ms) if apply_row else 0.0)
        full_ratios.append((full_row.median_ms / native.median_ms) if full_row else 0.0)

    if ratio_commands:
        fig, ax = plt.subplots(figsize=(14, max(6, len(ratio_commands) * 0.35)))
        y_pos = list(range(len(ratio_commands)))
        height = 0.35
        ax.barh(
            [y - height / 2 for y in y_pos],
            apply_ratios,
            height=height,
            label="vsh_apply",
            color=MODE_COLORS["vsh_apply"],
        )
        ax.barh(
            [y + height / 2 for y in y_pos],
            full_ratios,
            height=height,
            label="vsh_full",
            color=MODE_COLORS["vsh_full"],
        )
        ax.axvline(1.0, color="#333333", linestyle="--", linewidth=1, label="parity (1.0x)")
        ax.set_yticks(y_pos)
        ax.set_yticklabels(ratio_commands)
        ax.set_xlabel("median ratio vs native (>1 = slower)")
        ax.set_title("Speed ratio vs native shell (median)")
        ax.legend(loc="lower right")
        fig.tight_layout()
        ratio_path = output_dir / "median_ratio_vs_native.png"
        fig.savefig(ratio_path, dpi=160)
        plt.close(fig)
        written.append(ratio_path)

    # 4) Heatmap-style overview
    heatmap_modes = [mode for mode in modes if any(by_key.get((cmd, mode)) for cmd in commands)]
    if heatmap_modes:
        import matplotlib.pyplot as plt  # noqa: PLC0415 — already imported above

        matrix = []
        for cmd in commands:
            row = []
            for mode in heatmap_modes:
                stats = by_key.get((cmd, mode))
                row.append(stats.median_ms if stats is not None else float("nan"))
            matrix.append(row)
        fig, ax = plt.subplots(figsize=(8, max(6, len(commands) * 0.3)))
        image = ax.imshow(matrix, aspect="auto", cmap="YlOrRd")
        ax.set_xticks(range(len(heatmap_modes)))
        ax.set_xticklabels(heatmap_modes, rotation=20, ha="right")
        ax.set_yticks(range(len(commands)))
        ax.set_yticklabels(commands)
        ax.set_title("median latency heatmap (ms)")
        for y in range(len(commands)):
            for x in range(len(heatmap_modes)):
                value = matrix[y][x]
                if value != value:  # NaN check
                    continue
                ax.text(x, y, f"{value:.1f}", ha="center", va="center", fontsize=7, color="#111111")
        fig.colorbar(image, ax=ax, label="ms")
        fig.tight_layout()
        heatmap_path = output_dir / "median_heatmap.png"
        fig.savefig(heatmap_path, dpi=160)
        plt.close(fig)
        written.append(heatmap_path)

    return written
