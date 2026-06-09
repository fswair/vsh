# vsh Performance Baseline

Frozen reference measurements before roadmap changes. Use
[`playground/compare_baseline.py`](../playground/compare_baseline.py) to diff post-change
runs against this baseline.

## Canonical baseline directory

`playground/reports/baseline-pre-roadmap-20260609-193413/`

| Field | Value |
|-------|-------|
| Git commit | `c60cfe5` |
| Generated | 2026-06-09T19:46 UTC |
| Model (agent) | `openrouter:google/gemini-3-flash-preview` |

## Harnesses

### Playground command benchmark

```bash
uv run python playground/benchmark_vsh_vs_native.py \
  --iterations 50 --file-count 20 --file-size 256 \
  --output-dir playground/reports/baseline-pre-roadmap-20260609-193413/playground \
  --no-plots
```

- 25 commands × 3 modes (`native`, `vsh_apply`, `vsh_full`)
- `vsh_apply` median: ~0.02–0.18× native
- `vsh_full` median: ~0.10–0.42× native

### Agent context comparison

```bash
uv run python examples/agent_context_comparison.py \
  --output-dir playground/reports/baseline-pre-roadmap-20260609-193413/agent-context
```

Single-run snapshot (stochastic): vsh 12.3s / 7,359 input tokens / 5 tool calls vs
native 9.3s / 4,135 tokens / 5 tool calls. Use `--runs 3` for median summaries.

## Compare after changes

```bash
uv run python playground/compare_baseline.py \
  --baseline playground/reports/baseline-pre-roadmap-20260609-193413 \
  --current playground/reports/post-faz0-<stamp>
```
