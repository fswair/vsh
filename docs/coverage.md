# Coverage contract

Captured: 2026-10-02, local macOS arm64; optimized Bash integration checkout, not a new release.

Coverage is a merge gate, not an inferred property of the test count. Python and Rust
use separate measurements because the CPython extension and supervised worker cross
process/runtime boundaries that one coverage runtime cannot merge honestly.

## Python release surface

The Python gate runs all native binding, runtime, CLI, and MCP/CodeMode tests with
line and branch measurement:

```bash
uv run pytest \
  tests/test_main.py \
  tests/test_native_binding.py \
  tests/test_native_runtime.py \
  tests/test_pydantic_ai_capability.py \
  tests/test_commit_judge.py \
  tests/test_python_surface.py \
  --cov=src/vsh --cov-branch --cov-report=term-missing --cov-fail-under=100
```

Current result: 182 tests, 664 statements, 178 branches, 100% line and 100% branch
coverage.

`pyproject.toml` omits only the generated/static version module. Every maintained
Python module in the release surface is included in line and branch measurement.

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
| Lines | 84.51% | 79% |
| Functions | 77.34% | 70% |
| Regions | 84.93% | 81% |

The measurement executed 296 Rust tests. The ignore expression affects the threshold
report, not test execution. `vsh-python`
is loaded and exercised by the Python/PyO3 suite. `vsh-worker` is exercised through
real subprocess protocol/isolation tests. These boundaries require their own
behavioral tests; parent-only zero coverage is not proof that they are untested.
The new `vsh-bash` protocol, parent gateway and worker code are **not** added to the
ignore expression. Stable LLVM reports regions, not Rust branch coverage; this run
does not establish 100% Rust branches.

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
Hook coverage additionally exercises immutable canonical evidence, read-only scope,
pending feedback, approval, hard-deny exclusion and fail-closed Python handler errors.
The Bash suite adds binary streams, profile rejection, durable preview/restart,
permission review and stale mode checks, caught policy denial, bounded evidence,
worker retirement and cancellation. Python tests also cover repeated cancellation,
unseen-preview cleanup, durable automatic-approval revocation and the commit-entry race.
The Pydantic AI tests register a real capability on `Agent`, execute every filesystem
tool, verify JSON-safe result projection, and preserve review feedback without adding a
new lifecycle state.

Judge tests use offline Pydantic AI models and exercise direct pending approval,
canonical before/after content, main-agent review/reject feedback, invalid evidence
references, incomplete or binary content, content-sharing authorization, bounded
input/concurrency, timeout, cancellation, provider failure and stale commit rejection.
They verify the integration contract, not a real model's judgment accuracy or
resistance to prompt injection.

The optimization additions include a 266,321-case path-matcher differential oracle,
compiled-pattern fast-path comparisons, portable path normalization oracle checks,
overlay prefix-sibling visibility and existing generated-operation replay checks.
The wildcard fix checks both compiled and general matchers against an independent
byte-DP oracle for all 116,281 component cases, including literal-star filenames.
Active-policy checks cover direct and hooked approval/commit after restart; injected
commit failures verify recovery after a policy change. Diff tests verify canonical
entries, digest and complete metrics, both lexical directions of lazy rename, and
metadata-only edits without content capture. State-log tests verify unchanged frame
bytes for optional artifact/approval fields and reject oversize appends before I/O.
Store tests cover existing-directory reuse without changing its contents,
non-directory rejection and concurrent creation; existing symlink and capability
relocation tests retain the filesystem trust boundary.
An ancestor-first point-lookup oracle covers missing nodes, tombstones and hidden
present nodes across generated operation sequences.
Snapshot tests preserve punctuation/root ordering; sibling-resolution
tests cover renamed, deleted and non-directory ancestors without resurrecting hidden
children. No evidence limit, security check or coverage exclusion was relaxed.
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
