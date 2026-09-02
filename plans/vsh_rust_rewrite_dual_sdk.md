# VSH Rust Rewrite v2 — Native Rust + PyO3 Python SDK

Status: authoritative implementation plan\
Source design SHA-256: `f024ac6db7a765a92e01a19c5858a3b6f3ec0a93404aaafcaaf62073c404cb0f`\
Baseline Python revision: `7c7d5cbeaa88f58b99a1cc23953a0ad3b6bc5d91` (`vsh 0.3.0`)\
Decision date: 2026-08-28

This document supersedes the packaging, public API, dependency, performance, and
phase-order portions of the original “VSH Rust Rewrite — Full System Design &
Implementation Plan”. Security invariants and the virtual-filesystem architecture from
that document remain authoritative unless this document strengthens them.

## Implementation status — 2026-08-29

| Phase | State | Evidence / remaining release work |
|---|---|---|
| 0–3 | Implemented | The corrected Python baseline candidate, immutable VFS, canonical diff, and supervised Monty 0.0.21 worker are recorded under `docs/rust-rewrite/`; freezing the candidate as a clean tag still requires explicit release authority. |
| 4–7 | Implemented | Protected-read policy, exact durable approvals, single-use recovery-aware committer, race/fault tests, same-workspace commit coordination, pinned workspace/runtime identities, bounded recovery reads, and fail-closed internal symlink handling are in Rust. |
| 8 | Implemented | The `vsh` public facade, `vsh-runtime` implementation, and `vsh._native` PyO3 surface share one version; Python result compatibility is checked before persistence/commit, blocking native work releases the GIL, and panics map into one catchable Python exception hierarchy. |
| 9 | Implemented | The optional FastMCP adapter exposes only `vsh_run`; preview promotion reuses the exact native transaction. |
| 10 | In progress | Paired native-Rust and PyO3 fat-LTO evidence closes preview-fsync and full-path snapshot metadata bottlenecks; the final post-hardening 100-sample rerun keeps distinguishable incremental PyO3 p50 at 1.3–16.5 µs, covers search over 10k files, 100-file rename/delete subtrees, a 5,050-node delete, 30 cold starts, and independent-runtime scaling. Equivalent clean frozen-baseline, supported-platform/worker-tree RSS, and adversarial performance runs remain release gates. |
| 11 | In progress | Isolated CPython 3.14 wheel and sdist install/import/commit, all eight registry crate archives, Rust advisory/license/source gates, the hash-locked Python CVE scan, 100% shipped-Python line/branch coverage, and Rust core coverage of 80.52% lines / 71.78% functions / 82.53% regions pass locally. A SHA-pinned build-only rehearsal plus gated 20-wheel dual-registry/provenance workflow is implemented; executing the hosted platform matrix and publishing still require CI environments, credentials, and an explicit release tag. |
| 12 | Deferred | No deferred optimization is admitted without new benchmark evidence. |

## 1. Product north star

VSH is one high-performance, validation-first execution engine with two first-class
language surfaces:

- Rust users consume a native crates.io library.
- Python users consume a PyPI wheel whose implementation is a thin PyO3 binding over
  the same Rust library.

There must never be a Python simulator and a Rust simulator. There is one semantic
implementation, one policy implementation, one transaction state machine, one
canonical diff implementation, and one committer: the Rust core.

The rewrite succeeds only if it simultaneously improves:

1. correctness and containment,
2. warm and cold latency,
3. throughput and bounded resource use,
4. token efficiency,
5. Rust and Python developer experience.

Passing functional tests while moving the old bottleneck into FFI conversion,
serialization, locking, SQLite, worker startup, or diff canonicalization is failure.

## 2. Explicit non-goals

- No second implementation of core behavior in Python.
- No command-specific simulator/executor pairs.
- No shell-compatible language or arbitrary host shell access.
- No JSON serialization between internal Rust stages.
- No async runtime in the deterministic core unless profiling proves it is necessary.
- No dependency added for convenience when `std` is clear and sufficient.
- No unknown, abandoned, unpinned, yanked, git-only, wildcard, or pre-release dependency.
- No MCP/agent UX work before VirtualFs correctness and zero-host-effect execution are
  proven.
- No performance claim without a reproducible benchmark artifact.
- No `abi3` packaging decision without measuring its cost against per-Python-version
  wheels.

## 3. Hard architecture decisions

### 3.1 Single Rust source of truth

```text
Rust caller ───────────────┐
                          │ typed Rust API
                          ▼
                    public VSH facade
                          │
Python caller             │
    │                     │
    └─ PyO3 boundary ─────┘
                          ▼
                transaction runtime
                          ▼
 Monty → VirtualFs → canonical diff → policy/judge → committer
```

The PyO3 crate may validate Python object shape, map errors, and convert return values.
It may not reimplement path normalization, filesystem semantics, policy, approval,
revalidation, recovery, or receipt generation.

### 3.2 Public packages

The intended release train contains:

| Surface | Registry | Role |
|---|---|---|
| `vsh` | crates.io | Primary native Rust SDK/facade |
| `vsh-python`, `vsh` import package | PyPI | PyO3-backed Python SDK and CLI |
| `vbash` | both | Implementation-free exact-version compatibility mirrors |

The historical crates.io `vsh` and `vbash` handles were transferred to the project and
verified under the `fswair` owner on 2026-09-02. `vsh-runtime` remains the implementation
crate while `vsh` is the stable application-facing facade. PyPI uses `vsh-python`
because the `vsh` distribution is unavailable; Python imports remain `vsh`. The old
`vbash` names exact-pin and mirror the matching primary package version without owning
another implementation.

Internal workspace crates started with `publish = false`. Packaging evidence selected
the following release shape before crates.io release:

1. internal crates needed by the public facade are published in dependency order with
   verified names, or
2. implementation modules are consolidated behind the public facade.

The implementation selects option 1: the seven internal libraries and the supervised
worker use project-owned, currently unallocated `vsh-*` names, exact lockstep
requirements, and `publish = ["crates-io"]`. This preserves compile-time boundaries and
avoids a high-risk source consolidation solely for registry mechanics. Recheck name
ownership immediately before the irreversible first publish.

### 3.3 Lockstep semantics and versioning

- Rust and Python releases share one VSH semantic version.
- Every Python wheel records the Rust core version and source revision.
- `vsh.__version__`, crate version, wheel metadata, and release tag must agree.
- A receipt generated through Rust and Python for the same transaction must be
  semantically identical.
- Schema/receipt changes require cross-language golden tests.

## 4. Workspace layout

```text
vsh/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── deny.toml
├── crates/
│   ├── vsh-types/       # VPath, IDs, states, public data contracts
│   ├── vsh-store/       # BlobStore, checksummed append-only transaction state
│   ├── vsh-vfs/         # immutable snapshot, COW overlay, ledger/read/write sets
│   ├── vsh-monty/       # typed Monty OS-call adapter and worker lifecycle
│   ├── vsh-policy/      # call policy, transaction policy, judge contract
│   ├── vsh-commit/      # revalidation, journal, recovery, verification
│   ├── vbash/           # orchestration implementation (`vsh-runtime`)
│   ├── vsh/             # primary public Rust facade
│   ├── vbash-compat/    # exact-version compatibility re-export of `vsh`
│   ├── vsh-python/      # PyO3 extension (`vsh._native`), crates.io publish disabled
│   └── vsh-worker/      # bounded, crash-isolated Monty worker executable
├── compat/vbash/        # metadata-only PyPI compatibility installer
├── src/vsh/             # thin Python package, stubs, CLI, optional MCP adapter
├── tests/
│   ├── adversarial/
│   ├── parity/
│   ├── integration/
│   └── fixtures/
├── benches/
└── plans/
```

Dependency direction:

```text
vsh-types → vsh-store → vsh-vfs → vsh-policy
     \           \          \          \
      └────────── vsh-commit + vsh-monty ─→ vsh-runtime ─→ vsh ─→ vbash
                                                    └─────→ vsh-python → Python MCP adapter

Monty crates → vsh-worker (separate supervised process; no core back-edge)
```

No core crate depends on PyO3, MCP, Click, Pydantic, or another adapter framework.

The current requirement-by-requirement verification, resolved hardening findings, and
remaining external release gates are recorded in
`docs/rust-rewrite/PLAN_VALIDATION.md`.
`vsh-python` depends inward on the `vsh` facade; neither facade nor runtime depends
outward on Python.

## 5. Native Rust API

The native API is typed, synchronous at the core boundary, and small:

```rust
pub struct Runtime { /* private */ }

pub struct RunRequest<'a> {
    pub code: &'a str,
    pub intent: Option<&'a str>,
    pub mode: RunMode,
    pub detail: ReceiptDetail,
    pub budget: ExecutionBudget,
}

impl Runtime {
    pub fn open(config: RuntimeConfig) -> Result<Self, VshError>;
    pub fn run(&self, request: RunRequest<'_>) -> Result<Receipt, VshError>;
    pub fn preview(&self, request: RunRequest<'_>) -> Result<Receipt, VshError>;
    pub fn discard_preview(&self, transaction: TransactionId) -> Result<bool, VshError>;
    pub fn approve(
        &self,
        transaction: TransactionId,
        principal: PrincipalId,
        issued_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Result<TransactionRecord, VshError>;
    pub fn commit(
        &self,
        transaction: TransactionId,
        now_unix_ms: u64,
    ) -> Result<Receipt, VshError>;
    pub fn recover(&self) -> Result<RecoveryReport, VshError>;
}
```

Rules:

- Public structs are semantic contracts, not storage row mirrors.
- Errors are typed and stable enough for exhaustive matching where appropriate.
- Internal IDs and digests use strongly typed newtypes.
- Large payloads are accessed by artifact/blob handle instead of copied into receipts.
- Default features stay minimal; optional observability or adapter features are opt-in.

## 6. PyO3 Python API

### 6.1 Binding structure

`vsh-python` builds `vsh._native` with PyO3. The pure-Python package contains:

- intentional re-exports,
- type stubs / `py.typed`,
- compatibility shims during migration,
- CLI composition where Python remains useful.

It contains no alternate execution engine.

Proposed Python contract:

```python
from vsh import Runtime, RunRequest, RunMode, Receipt

runtime = Runtime.open(workspace="/path/to/workspace")
receipt = runtime.run(
    RunRequest(
        code="from pathlib import Path\nPath('out.txt').write_text('ok')",
        mode=RunMode.AUTO,
    )
)
```

### 6.2 FFI performance rules

- Convert each request once at the boundary.
- Call Rust with borrowed UTF-8/bytes where lifetime rules permit.
- Release the GIL with `Python::detach`/the supported PyO3 equivalent while Monty,
  VirtualFs, policy, revalidation, and commit work runs without Python objects.
- Reacquire the GIL only to construct the final Python result or exception.
- Never call back into Python from the core transaction hot path.
- Do not move internal data through `dict`, JSON, Pydantic, or serde merely to cross
  internal Rust layers.
- Return compact native Python objects; expose large bodies through explicit artifact
  reads supporting buffer/bytes access.
- Provide a batch entry point only when it represents one transaction and reduces
  boundary crossings without weakening policy semantics.

### 6.3 Wheel policy

Start with per-Python-version wheels for Python 3.11+ to preserve the fastest CPython
API path. Benchmark `abi3-py311` separately. Adopt abi3 only if:

- semantic parity remains exact,
- binding overhead stays within the accepted budget,
- supported platforms remain complete,
- the wheel reduction materially improves release operations.

Build and publish with Maturin. Source distributions must either build deterministically
with the pinned Rust toolchain or be omitted until that path is proven.

## 7. Simulation and transaction core

The core flow remains:

```text
untrusted Monty program
  → typed OsFunctionCall
  → VirtualFs over immutable BaseSnapshot + transaction Overlay
  → EffectLedger + ReadSet + WriteSet
  → CanonicalDiff
  → deterministic policy
  → AutoApprove | Deny | Escalate(fresh judge)
  → atomic reservation
  → dependency-only revalidation
  → crash-recoverable trusted commit
  → post-commit verification
  → compact Receipt
```

Simulation is actual execution against virtual state, not a prediction later replayed
with different semantics. The host writer is the committer and no other component.

## 8. Security invariants

The original ten invariants remain mandatory:

1. complete mediation,
2. no live-host fallback,
3. one filesystem semantics,
4. no replay of approved commands,
5. approval bound to the exact transaction artifact,
6. single-use commit,
7. least-authority reads,
8. bounded execution,
9. fail closed,
10. one trusted host writer.

Additional dual-SDK invariants:

11. Python cannot bypass or broaden a native capability.
12. Rust and Python surfaces produce the same policy decision and canonical receipt.
13. A Python exception cannot leave a transaction reserved or partially committed.
14. Panics never unwind across the FFI boundary; they become a fail-closed internal
    error after recovery obligations are recorded.
15. No Python object or GIL dependency exists inside the committer.

## 9. Dependency admission policy

Dependencies are deny-by-default. A crate may be introduced only with a dependency
admission record containing:

- exact crate and version,
- direct purpose and why `std` is insufficient,
- official crates.io and source repository links,
- non-yanked stable release evidence,
- active-maintenance evidence,
- license,
- MSRV,
- enabled features and disabled default features,
- known RustSec/CVE state at the review timestamp,
- transitive dependency delta,
- replacement/removal strategy.

Required policy:

- Exact pins use `=x.y.z` in `[workspace.dependencies]`.
- `Cargo.lock` is committed and CI uses `--locked`/`--frozen` as appropriate.
- Git, branch, path-to-external-repo, wildcard, broad range, and unreviewed pre-release
  dependencies are forbidden in release artifacts.
- Default features are disabled unless every default feature is justified.
- Sources are crates.io plus workspace path dependencies only.
- Licenses are allowlisted in `deny.toml`.
- Duplicate major versions and unexpected native libraries fail review.
- Unsafe code in VSH crates is forbidden by default. Necessary FFI unsafe code is
  isolated in `vsh-python`, documented, and reviewed.
- `cargo audit` and `cargo deny check` run in CI and before release.
- An advisory failure blocks merge/release; ignores require a bounded expiry, rationale,
  and owner.
- Dependency updates happen in explicit audit PRs, never incidentally.

“No CVE” means no known published advisory in the checked sources at that timestamp; it
does not claim that undiscovered vulnerabilities cannot exist.

### 9.1 Initial verified pins

Verified on 2026-08-29 against official registry/source metadata and refreshed advisory
databases:

| Dependency/tool | Exact version | Scope | Status |
|---|---:|---|---|
| Rust toolchain | `1.95.0` | build/MSRV initially aligned with Monty | active stable toolchain |
| `monty` | `=0.0.21` | interpreter core | Pydantic-maintained, stable, non-yanked |
| `monty-types` | `=0.0.21` | typed calls/resource contracts | same release train |
| `monty-proto` | `=0.0.21` | typed worker protocol and lossless Monty value → native Python conversion | same release train; worker feature disabled |
| `monty-alloc` | `=0.0.21` | worker-wide memory ceiling | same release train; only `exit-code` enabled |
| `pyo3` | `=0.29.2` | Python binding | PyO3-maintained, stable |
| `blake3` | `=1.8.7` | blob/snapshot/diff identity | official active BLAKE3 implementation; `std` only |
| `cap-std` | `=4.0.3` | capability-rooted host filesystem access | Bytecode Alliance-maintained, stable, non-yanked; no default features |
| `postcard` | `=1.1.3` | exact Monty result encoding inside durable artifacts | stable wire format; `alloc` only and no default features |
| Maturin | `==1.15.0` | Python build/publish tool | PyO3-maintained, stable |
| FastMCP | `==3.4.7` | optional Python MCP adapter | actively maintained stable release; absent from core installs |
| cargo-audit | `=0.22.2` | CI/release tool | RustSec audit client |
| cargo-deny | `=0.20.2` | CI/release tool | advisory/license/source policy |
| pip-audit | `==2.10.1` | CI/release tool | PyPA advisory client; ephemeral exact-pin |

This table is not permission to add adjacent packages or features. The published
`monty-runtime`/`monty-proto worker` graph was explicitly rejected because it selects
yanked `chacha20 0.10.0` through the type-checking stack. VSH uses a bounded local
`std` pool around a minimal public-API worker instead. Every new runtime crate still
needs an admission record and a clean full-lockfile audit after resolution.
The optional/development Python lock resolves the advisory-fixed
`cryptography==50.0.1`, `mcp==1.29.1`, `pydantic-settings==2.15.0`, and
`starlette==1.6.0`; the strict hash-locked pip-audit scan reports no known
vulnerability.

## 10. Performance architecture

### 10.1 Measured stages

Every benchmarkable transaction records monotonic stage timings without allocating
strings in the hot path:

```text
request_decode
snapshot_acquire
worker_checkout
monty_execute
vfs_calls
diff_freeze
policy
judge (when present)
reservation
revalidation
commit
verification
receipt_encode
ffi_total (Python only)
```

Instrumentation is cheap counters/timestamps in normal builds and richer tracing only
behind a feature. The measurement system must not become the dominant cost.

### 10.2 Bottleneck rules

- Profile before optimizing.
- A stage responsible for more than 25% of warm p50 or p99 gets an explicit profile and
  ownership decision.
- No global mutex may cover Monty execution, VFS work, diff generation, or Python result
  construction.
- Durable state-store locks stay short and never wrap virtual execution; the default
  checksummed append log uses only `std`; at its explicit byte bound it rewrites only
  latest records into an inactive slot and switches through a fixed-size,
  checksummed double-buffered control record.
- Snapshot metadata is eager; content is lazy, immutable after capture, and cached by
  content hash.
- Overlay mutations are COW and proportional to touched state, not workspace size.
- Commit revalidates ReadSet/WriteSet dependencies, not the entire workspace.
- Directory enumeration uses stable digests and bounded memory.
- Worker reuse is allowed only after a successful clean reset; resource-limit or panic
  exits discard the worker.
- Large results spill to BlobStore/ArtifactStore instead of crossing FFI or model
  boundaries inline.

### 10.3 Performance acceptance budgets

Phase 0 records absolute baselines. Phase 1 then freezes hardware-normalized budgets.
Release requires all of the following:

- Native Rust p50 and p99 simulation latency beat the frozen Python baseline for every
  equivalent scenario; no accepted scenario may regress.
- Geometric-mean warm simulation speedup is at least 2x over the Python baseline.
- Python-binding p50 overhead over the same native Rust call is at most 10% and 100 µs,
  whichever is stricter after subtracting unavoidable Python test harness cost.
- Python-binding p99 has no unexplained tail amplification greater than 15%.
- Ten-file and twenty-edit workloads perform one Python→Rust transaction call, not one
  FFI call per filesystem operation.
- Peak memory and overlay bytes remain bounded by `ExecutionBudget` and are reported.
- Throughput scales across independent runtimes without a process-global serialization
  point.
- Benchmark variance and sample count are reported; single-run wins do not count.

If Phase 0 evidence shows a provisional number is physically misleading, update the
budget before implementation work relies on it and record the evidence. Do not relax a
budget merely because the implementation misses it.

## 11. Benchmark matrix

Each scenario runs through native Rust, PyO3 Python, and the frozen Python baseline when
the old implementation supports it:

```text
read 10 files
search 10k files
edit 1 file
edit 20 files
rename subtree
delete subtree
massive delete
stale workspace
symlink tricks
double execute
Monty worker cold start
Monty worker warm reuse
no-op/compact receipt (FFI floor)
large artifact spill
parallel independent runtimes
```

Report p50, p95, p99, min/max, throughput, allocations where available, peak RSS,
bytes copied across FFI, filesystem calls, durable state-store time, and receipt size. Store raw
machine-readable results plus a human-readable comparison.

## 12. Migration and compatibility

The Python implementation is a behavioral oracle and regression corpus, not code to
translate mechanically.

Preserve:

- user-visible safe behavior that matches the new invariants,
- command/use-case corpus,
- path and protected-read tests,
- policy expectations,
- artifact and receipt semantics worth retaining,
- benchmark scenarios,
- Python import/CLI compatibility explicitly covered by the migration matrix.

Do not port:

- command-specific simulator switches,
- `PredictedEffects` as source of truth,
- snapshot-advance approximations,
- real-command dispatcher semantics,
- JSON PlanStore persistence,
- per-command `execution_reason`,
- Python hot-path orchestration.

Compatibility shims must call Rust and carry a deletion milestone. They may not hide a
second engine.

## 13. Implementation phases

### Phase 0 — Freeze and evidence

Deliver:

1. tag/release identity for the clean Python baseline,
2. current test and security-regression corpus manifest,
3. latency/token/memory benchmark artifacts,
4. threat model,
5. guarantees/non-guarantees document,
6. dependency admission policy and initial audit record,
7. native/Python parity schema.

Gate: no Rust behavior work begins until the baseline is reproducible or every missing
measurement is explicitly marked with a reason and owner.

### Phase 1 — Workspace and dual-SDK skeleton

Implement:

- pinned Rust 1.95 toolchain and workspace policy,
- `vsh-types`, `vsh-store` skeleton, `vsh-runtime`, `vsh`, and `vsh-python`,
- VPath, BlobId, SnapshotId, TransactionId, TransactionState, and error model,
- checksummed, bounded two-slot file store using `std`; a database dependency is
  admitted only if cross-process contention/compaction evidence requires it,
- PyO3 import/version smoke test,
- native/Python version parity test,
- cargo fmt/clippy/test/doc, audit, deny, Maturin build gates.

No fake simulator is added to make the bindings look complete.

### Phase 2 — Immutable snapshot and VirtualFs

Implement BaseSnapshot, lazy content capture, BlobStore, Overlay, VirtualFs,
EffectLedger, ReadSet, WriteSet, and CanonicalDiff. No host commit.

Property gate: applying arbitrary generated VFS operations and then applying the
canonical diff to an independent model produces the same final state.

Performance gate: operation cost scales with touched state; snapshot/content metrics
match the budgets.

### Phase 3 — Monty typed OS-call integration

- Depend only on exact admitted Monty 0.0.21 crates.
- Use the public typed OS-call seam; do not fork or add VSH-specific code to Monty.
- Provide zero host mounts, synthetic environment, bounded output, and resource limits.
- Map every V0 filesystem call to VirtualFs.
- Measure worker cold start, checkout, reset, and discard.

Gate: arbitrary supported Monty filesystem programs produce an exact virtual final
state and zero host effects.

Current status (2026-08-29): delivered. The complete 0.0.21 typed filesystem enum maps
into `VirtualFs`; production `Runtime` execution crosses a supervised, exact-versioned
worker boundary. The parent retains typed call dispatch, applies independent memory,
wall-time, event, frame, result, output, path, and I/O limits, and reuses only cleanly
reset workers. The official worker feature was rejected because its graph includes a
yanked crate; the replacement uses Monty's public typed run API without a fork or new
runtime dependency. Adversarial isolation tests pass. Release-mode warm/cold latency
is recorded as warm distributions plus 30 independent cold samples; peak RSS and
cross-platform baselines remain measurement tasks, not isolation blockers. See
`docs/rust-rewrite/PHASE_3_MONTY.md`.

### Phase 4 — Read capability and deterministic policy

Add pre-call protected-read policy, secret patterns, output-as-capability limits, and
transaction-level `Deny | AutoApprove | Escalate`. Fuzz and adversarial tests are
mandatory.

### Phase 5 — Approval and persistent state machine

Add durable CAS transitions, transaction digest/artifact binding, fresh judge interface,
expiry, and single-use reservation. The default implementation is a bounded,
checksummed append-log file store with short cross-process locks, torn-tail repair,
and crash-safe bounded two-slot compaction.
The judge can narrow an escalation but never reverse a deterministic deny. A database
backend remains an evidence-gated option, not a baseline dependency.

### Phase 6 — Trusted committer

Implement dependency-only revalidation, capability-rooted host access, staging,
quarantine, journal, recovery, post-commit verification, and fault injection at every
journal boundary.

### Phase 7 — Concurrency and crash testing

Cover overlapping transactions, read/write conflicts, rename races, stale snapshots,
double commit, worker crash, binding exceptions, interpreter shutdown, and process
restart. Prove there is no global serialization bottleneck across independent runtimes.

### Phase 8 — Stable native and PyO3 APIs

Finalize `vsh` native facade and `vsh._native`; generate/type-check Python stubs; add
cross-language golden tests; preserve only approved compatibility shims; document Rust
and Python quick starts.

### Phase 9 — MCP/model surface

Expose one normal tool, `vsh_run`, backed by the same runtime. Keep compact receipts,
preview mode, and artifact handles. Do not expose internal lifecycle steps to the model.

### Phase 10 — Performance closure

Run the complete benchmark matrix; profile every failed budget; remove old bottlenecks
without adding FFI, allocation, lock, database, startup, or serialization bottlenecks.
Compare native Rust, PyO3 Python, and frozen Python baseline in one report.

### Phase 11 — Packaging and release

- crates.io package dry-run and API docs,
- Maturin wheels for the supported Python/platform matrix,
- install/import/CLI tests from built artifacts,
- exact version consistency,
- cargo audit/deny and Python vulnerability scan,
- reproducible lockfiles and provenance,
- release notes and migration guide.

### Phase 12 — Deferred optimizations

Only benchmark evidence may promote:

- VSH high-level intrinsics,
- Monty checkpointing,
- persistent/Merkle snapshots,
- abi3 wheels,
- more elaborate worker pools.

Correctness and the measured hot path come first.

## 14. Release acceptance

Security:

- all original V0 adversarial tests pass,
- Rust/Python parity tests pass,
- no known unwaived RustSec/CVE advisory in the locked graph,
- license/source/duplicate policy passes,
- crash recovery passes at every injected boundary,
- secrets never cross into Monty or Python receipts without explicit capability.

Correctness:

- one filesystem semantics,
- exact canonical diff,
- deterministic policy parity,
- single-use commit,
- post-commit verification,
- stable typed errors in Rust and mapped exception hierarchy in Python.

Performance:

- all Phase 10 budgets pass on declared hardware,
- no stage exceeds the bottleneck threshold without an accepted evidence-backed reason,
- binding overhead and tail latency pass,
- benchmark artifacts are reproducible and versioned.

Product:

- native Rust and PyPI install paths work from built artifacts,
- both quick starts perform the same real workflow,
- docs state guarantees and non-guarantees precisely,
- no Python fallback engine remains,
- release versions are in lockstep.

## 15. Decision ledger

Decision: one Rust core serves both languages.\
Reason: semantic parity, lower latency, and removal of duplicate simulator/executor
implementations.\
Source: user direction, 2026-08-28.\
Invalidates if: none within this rewrite; a second core requires a new product decision.

Decision: Python uses PyO3.\
Reason: direct typed binding to the native engine without subprocess or JSON overhead.\
Source: user direction, 2026-08-28.\
Invalidates if: PyO3 cannot meet security or measured latency requirements and a proven
alternative is approved.

Decision: dependencies are exact-pinned and deny-by-default.\
Reason: reproducibility, supply-chain control, and avoidance of abandoned/unknown crates.\
Source: user direction, 2026-08-28.\
Invalidates if: registry policy prevents an exact pin for a required dependency; the
exception must be explicit and cannot silently widen the range.

Decision: performance is a release invariant, not a cleanup phase.\
Reason: the rewrite exists to remove latency and throughput bottlenecks without moving
them to new layers.\
Source: user direction, 2026-08-28.\
Invalidates if: correctness/security requires a measured tradeoff; document the cost and
obtain explicit approval.

Decision: Maturin/PyO3 initial verified versions are 1.15.0/0.29.2 and Monty is 0.0.21.\
Reason: latest stable official releases observed on the decision date.\
Source: official PyPI/crates.io metadata and Pydantic/PyO3 repositories.\
Invalidates if: a newer stable release passes the full admission and compatibility gate
before the dependency is first committed.

Decision: the baseline durable transaction store is dependency-free.\
Reason: the checksummed append log provides bounded recovery and atomic CAS; its
fixed-size two-slot control record switches compacted generations only when the byte
ceiling is reached, without placing a database, connection pool, or serialization
layer on the transaction hot path.\
Source: implementation and contention tests, 2026-08-28.\
Invalidates if: reproducible multi-process contention or compaction measurements fail
the release budgets; any replacement still requires full dependency admission.
