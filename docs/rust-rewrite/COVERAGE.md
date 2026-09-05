# VSH merge coverage contract

Captured: 2026-09-05

Coverage is a merge gate, not an inferred property of the test count. Python and Rust
use separate measurements because the CPython extension and supervised worker cross
process/runtime boundaries that one coverage runtime cannot merge honestly.

## Python release surface

The Python gate runs all native binding, runtime, CLI, and MCP/CodeMode tests with
line and branch measurement:

```bash
uv run pytest \
  tests/test_native_binding.py \
  tests/test_native_runtime.py \
  tests/test_python_surface.py \
  --cov=src/vsh --cov-branch --cov-report=term-missing --cov-fail-under=100
```

Current result: 48 tests, 179 statements, 40 branches, 100% line and 100% branch
coverage.

`pyproject.toml` omits only code that Maturin excludes from the wheel, plus the static
version literal. The legacy Python engine remains in the repository as a migration
oracle but is neither shipped nor counted as release-surface coverage. The release
artifact validator independently rejects legacy engine paths in wheels and sdists.

## Rust core

CI installs exact `cargo-llvm-cov =0.9.0` with `--locked` and runs every workspace
crate, feature, target, and test under stable Rust 1.95 coverage instrumentation:

```bash
cargo llvm-cov \
  --workspace --all-features --all-targets --locked --summary-only \
  --ignore-filename-regex '(vsh-python|vsh-worker)' \
  --fail-under-lines 79 \
  --fail-under-functions 70 \
  --fail-under-regions 81
```

Current stable-toolchain core result:

| Metric | Measured | Merge floor |
|---|---:|---:|
| Lines | 81.14% | 79% |
| Functions | 73.43% | 70% |
| Regions | 83.01% | 81% |

The measurement executed 154 Rust tests. The ignore expression affects the threshold
report, not test execution. `vsh-python`
is loaded and exercised by the 48 Python/PyO3 tests. `vsh-worker` is exercised through
ten real subprocess protocol/isolation tests. Both report zero when measured only by
the parent `cargo test` profile because they execute in a CPython runtime or a child
process; counting those zeros as untested Rust core would misstate both boundaries.

Rust 100% is not a merge target. Mutually exclusive Unix/Windows paths, injected I/O
failures, child-process code, and the CPython extension cannot all be represented
honestly by one stable-toolchain parent-process report. Chasing a headline number by
removing defensive branches or counting generated/subprocess code as covered would
weaken the signal. Instead, critical invariants have explicit behavioral tests:
single-use reservation finalization, every durable commit boundary, stale writes,
workspace/runtime relocation, internal symlink replacement, bounded directory growth,
checksummed state corruption, worker frame/output limits, GIL release, and Python panic
translation. The active-snapshot function tests additionally cover shared `pathlib`
visibility, recursive mutation preflight, bounded discovery, Unicode search offsets,
iterative deep glob matching, and typed call-frame sizing. The aggregate floor prevents
broad regressions; these tests protect the
high-risk contracts even where platform error branches remain unexecuted locally.

The optimization additions include a 94,501-case policy-matcher differential oracle,
compiled-pattern fast-path comparisons, portable path normalization oracle checks,
overlay prefix-sibling visibility and existing generated-operation replay checks.
Python acceptance also executes fixture-owning SDK/MCP/separate-process CLI recipes,
the actual first-run documentation block and multibyte Unicode output truncation.
APFS rejects invalid UTF-8 filename fixtures before snapshot capture; that platform
condition is explicit rather than mistaken for a runtime failure.

Rust branch coverage is not claimed: `cargo-llvm-cov --branch` remains nightly-only and
unstable. VSH keeps its production and coverage compiler pinned to stable Rust 1.95,
and uses stable region coverage plus explicit adversarial behavioral tests instead of
silently adding a nightly toolchain. Python branch coverage remains a hard 100% gate.

Coverage floors may only rise or stay fixed. Lowering or expanding an omission requires
an evidence-backed plan change and review.
