# vsh playground

Ad-hoc experiments and performance benchmarks. Not part of the installed package.

## vsh vs native benchmark

Measures **all 25 vsh command scenarios** (23 command models; `echo` and `sed` have stdout/write variants).

Each command/mode pair runs **5 iterations by default**; the report highlights **median** with **min/max** range.

| Mode | What it measures |
|------|------------------|
| `native` | `subprocess` shell command on the workspace |
| `vsh_apply` | `apply_command` only (real filesystem dispatch) |
| `vsh_full` | Full agent path: snapshot → simulate → approve → execute |

```bash
# default: 5 iterations, 50 files, writes report + plots
uv run python playground/benchmark_vsh_vs_native.py

# custom workload
uv run python playground/benchmark_vsh_vs_native.py --iterations 5 --file-count 200

# JSON to stdout + files under playground/reports/<timestamp>/
uv run python playground/benchmark_vsh_vs_native.py --json
```

### Outputs

Each run writes to `playground/reports/<timestamp>/`:

- `results.json` — raw stats + per-iteration samples
- `report.md` — markdown table (median/min/max)
- `plots/median_latency.png` — grouped median bars
- `plots/range_<mode>.png` — median with min/max whiskers
- `plots/median_ratio_vs_native.png` — speed ratio vs native
- `plots/median_heatmap.png` — overview heatmap

Requires `matplotlib` (`uv sync` installs it via the `dev` group).

`rg` native timing is skipped when ripgrep is not installed.
