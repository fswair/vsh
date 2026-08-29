# Rust rewrite Phase 0 baseline

Captured: 2026-08-28\
Python version: VSH 0.3.0\
Baseline revision candidate: `7c7d5cbeaa88f58b99a1cc23953a0ad3b6bc5d91`

## Baseline status

The candidate revision contained an MCP import regression: CodeMode modules imported
`mcp.server.FastMCP`, while the project uses the third-party `fastmcp.FastMCP` API and
passes its `version=` constructor argument. Eleven test modules failed during collection.

The Phase 0 working tree contains the narrow correction:

- `src/vsh/mcp/codemode_server.py`, `prompts.py`, and `surface.py` import
  `fastmcp.FastMCP`.
- `resources.py` uses the concrete supported `fastmcp.resources.function_resource`
  import instead of a fallback that mixes two APIs.

A release tag must point to a commit containing this correction. The baseline is not
considered immutable until that clean commit/tag exists.

## Verification

Command:

```bash
uv run pytest -q
```

Result after the correction:

```text
441 passed
100.00% line and branch coverage
8.68 seconds pytest-reported runtime
```

The count includes eight native PyO3 binding smoke and path-contract tests introduced
by the Phase 1 skeleton. The original corrected Python baseline remains 433 tests; no
existing test was removed or relaxed.

There are 3,488 third-party `pathspec` deprecation warnings. They do not fail the
current gate, but warning elimination must be tracked separately; the Rust rewrite may
not copy a broad warning suppression.

Focused MCP regression command:

```bash
uv run pytest \
  tests/test_mcp.py \
  tests/test_mcp_codemode.py \
  tests/test_agent_codemode_mcp.py \
  -q --no-cov
```

Result: 34 passed.

## Latency baseline

Artifact directory:

```text
playground/reports/rust-rewrite-python-v0.3.0/
```

Command:

```bash
uv run python playground/benchmark_vsh_vs_native.py \
  --iterations 25 \
  --file-count 50 \
  --file-size 512 \
  --output-dir playground/reports/rust-rewrite-python-v0.3.0 \
  --no-plots
```

Environment:

- Apple M1, 8 cores, 8 GB RAM
- macOS 26.1 arm64
- Python 3.12.10
- FastMCP 3.4.2
- Pydantic 2.13.4
- pydantic-monty 0.0.18

Matrix:

- 25 commands
- 25 samples per command/mode
- modes: native subprocess, VSH apply-only, VSH full lifecycle

Aggregate median statistics:

| Mode | Median of command medians | Geometric mean of command medians |
|---|---:|---:|
| Native subprocess | 5.647 ms | 6.007 ms |
| VSH apply-only | 0.151 ms | 0.166 ms |
| VSH full lifecycle | 3.755 ms | 3.747 ms |

Notable full-lifecycle medians:

| Command | Median |
|---|---:|
| `cat` | 2.241 ms |
| `grep` | 5.944 ms |
| `rg` | 8.492 ms |
| `cp` | 4.851 ms |
| `mkdir` | 6.351 ms |
| `rm` | 6.253 ms |
| `sed_inplace` | 5.294 ms |

The full lifecycle has visible long-tail outliers in several cases. The new benchmark
must add p95/p99 and stage attribution; median-only improvement is insufficient.

## Token baseline

Existing recorded artifacts are retained under:

```text
playground/reports/baseline-pre-roadmap-20260609-193413/
playground/reports/current-verified-20260610-002353/
```

The verified recorded agent run reached one `apply_batch` call. A fresh live-model run
is intentionally not part of the deterministic Phase 0 command because it can incur
external cost and stochastic drift. Refreshing it requires an explicit recorded model,
provider, prompt, and run count.

## Required expansion before performance closure

The existing command benchmark does not yet cover the full rewrite acceptance matrix.
Add deterministic cases for:

- read 10 files,
- search 10k files,
- edit 20 files in one transaction,
- rename/delete subtrees,
- massive delete,
- stale workspace and symlink races,
- double execute,
- Monty cold/warm worker cost,
- Python FFI floor and bytes copied,
- independent-runtime concurrency,
- peak RSS and overlay size.

## Phase 0 open item

- Create the clean baseline commit/tag after the import regression correction and the
  Phase 0 documents are reviewed. Codex will not create a commit or tag without explicit
  user direction.
