# vsh Pre-Roadmap Baseline

Frozen reference measurements before roadmap changes (locking, gitignore snapshot, apply_patch, etc.).

## Metadata

| Field | Value |
|-------|-------|
| **generated** | 2026-06-09T19:46 UTC |
| **git commit** | `c60cfe5` (`c60cfe5e5784a2c21a32f5d52066a5b46e049f71`) |
| **model** (agent) | `openrouter:google/gemini-3-flash-preview` |
| **harness note** | `playground/benchlib/runner.py` injects `execution_reason` for `vsh_full` mutations |

## Playground command benchmark

**Script:** `uv run python playground/benchmark_vsh_vs_native.py`

**Params:** `--iterations 50 --file-count 20 --file-size 256 --modes native,vsh_apply,vsh_full`

**Output:** [`playground/`](playground/) (`results.json`, `report.md`)

### Aggregate median ratios (vsh / native, lower = faster)

| Mode | Median across 25 commands | Slowest command |
|------|--------------------------:|-----------------|
| `vsh_apply` | ~0.05x | `grep` 0.18x, `find` 0.11x |
| `vsh_full` | ~0.25x | `echo_write` 0.42x, `mv` 0.38x |

**Highlights:**

- `vsh_apply` median: **0.02x–0.18x** native (grep/rg slowest at ~0.18x / 0.13x)
- `vsh_full` median: **0.10x–0.42x** native (mutations ~0.27x–0.42x overhead from snapshot+simulate+approve)
- `grep` vsh_full: **2.37 ms** vs native **7.03 ms** (0.34x)
- `rg` vsh_full: **2.18 ms** vs native **9.28 ms** (0.24x)

Local FS dispatch is fast; full pipeline adds ~0.5–1.5 ms for reads, ~1.5–2.0 ms for mutations.

## Agent context comparison

**Script:** `uv run python examples/agent_context_comparison.py`

**Output:** [`agent-context/`](agent-context/) (`comparison.json`, `comparison.md`)

### Results (single run — model non-deterministic)

| Metric | vsh CodeMode | native FS | vsh vs native |
|--------|-------------:|----------:|--------------:|
| Wall time | 12,254 ms | 9,254 ms | **-32%** (vsh slower) |
| Input tokens | 7,359 | 4,135 | **-78%** (vsh more) |
| Output tokens | 1,031 | 313 | -229% |
| Total tokens | 8,390 | 4,448 | -89% |
| Model requests | 6 | 7 | — |
| Tool calls | 5 | 5 | — |
| History bytes | 19,141 | 14,968 | -28% |
| Tool return bytes | 2,458 | 338 | — |
| Validation | PASS | PASS | both passed |

### vsh tool calls (this run)

5× `apply_batch` + 1× `apply` — model retried due to wrong param aliases (`directory`, `file_path`, `root_directory` instead of `path`/`root_dir`). Alias normalization at `c60cfe5` covers `dir`, `root_dir`, `dest` but not all variants.

**Important:** Agent benchmark is **stochastic**. A prior run (`agent-context-20260609-101614`) with better alias handling achieved 1 `apply_batch`, 1,298 input tokens, 5.6s. This baseline captures **committed-state variance** — compare future runs against this directory, not the best-case run.

## How to reproduce

```bash
BASELINE_DIR="playground/reports/baseline-pre-roadmap-20260609-193413"

uv run python playground/benchmark_vsh_vs_native.py \
  --iterations 50 --file-count 20 --file-size 256 \
  --output-dir "$BASELINE_DIR/playground" --no-plots

uv run python examples/agent_context_comparison.py \
  --output-dir "$BASELINE_DIR/agent-context"
```

## Future comparison checklist

When re-running after roadmap changes, compare:

1. **Playground:** per-command median ms, `vsh_full`/`vsh_apply` ratios, total bench wall time
2. **Agent:** input tokens, tool call count, model requests, wall time, validation pass rate
3. **Scale fixtures** (roadmap A5): add 1k/10k file runs alongside this 20-file baseline
4. **Commit hash** and harness changes documented in comparison report metadata
