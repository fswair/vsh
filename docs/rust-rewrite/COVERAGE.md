# VSH merge coverage contract

Captured: 2026-08-29

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

Current result: 38 tests, 179 statements, 40 branches, 100% line and 100% branch
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
| Lines | 80.52% | 79% |
| Functions | 71.78% | 70% |
| Regions | 82.53% | 81% |

The measurement executed 134 Rust tests. The ignore expression affects the threshold
report, not test execution. `vsh-python`
is loaded and exercised by the 38 Python/PyO3 tests. `vsh-worker` is exercised through
eight real subprocess protocol/isolation tests. Both report zero when measured only by
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
translation. The aggregate floor prevents broad regressions; these tests protect the
high-risk contracts even where platform error branches remain unexecuted locally.

Rust branch coverage is not claimed: `cargo-llvm-cov --branch` remains nightly-only and
unstable. VSH keeps its production and coverage compiler pinned to stable Rust 1.95,
and uses stable region coverage plus explicit adversarial behavioral tests instead of
silently adding a nightly toolchain. Python branch coverage remains a hard 100% gate.

Coverage floors may only rise or stay fixed. Lowering or expanding an omission requires
an evidence-backed plan change and review.
