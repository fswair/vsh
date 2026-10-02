# Bashkit public integration — completion evidence

Captured: 2026-10-01–02, local macOS 26.1 arm64; CPython 3.14.6. Baseline commit:
`8550facbe94485cb12bfe007fbb3575a168b6afc`; implementation is the uncommitted checkout.
Package versions remain 0.5.0. This file is not release or push authorization.

## Review decision — 2026-10-02

The four concrete review findings below are fixed and approved against local macOS
execution evidence. No source changes were delegated during this correction pass.
This is **not unconditional merge/release approval**: the existing all-workload
performance gate and unexecuted hosted platform matrix remain open. No commit, push
or release was performed.

Completion follow-up: CI's optional worker-override assumption is fixed; bounded
Monty prefix reads avoid one redundant read without relaxing validation. Final local
verification passes **282 Rust tests**, **175 Python tests / 100% line and branch
coverage**, unchanged Rust coverage floors, Clippy, Python lint/types, package smoke
checks and strict documentation checks. Short Circuit's independent read-only review
found no new issue in that transport change, but independently reproduced the failed
performance gates. Seven of nine workloads fail the latest paired all-case gate.
**The requested overall completion remains blocked**, not silently approved: a clean
controlled performance investigation is still required, and running exact-checkout
hosted CI requires the outstanding authorization to commit/push a review branch.

| Finding | Correction | Regression evidence |
| --- | --- | --- |
| P1: same-target symlink replacement could overwrite without a recovery backup | Only regular-file metadata updates skip quarantine. Symlink replacements always preserve the old entry; staged link metadata is validated before any workspace operation | macOS mode-600/mode-700 replacement fails before mutation; representable same-target replacement succeeds; injected intent/applied/done crashes restore both original links and modes; all 37 commit tests pass |
| P1: synchronous MCP execution could outlive protocol cancellation and auto-commit | Server registers a cancellable async adapter; synchronous Python helper remains supported. Both initial execution and exact-transaction promotion use native cancellation arbitration | Actual FastMCP client sends `notifications/cancelled` during a sleeping Bash auto-run: native work joins and no late file appears. Registered-tool tests cover initial and resumed commits after commit entry; repeated cancellation preserves their actual committed result |
| P2: relative Bash worker paths failed after the child changed directory | Resolve the trusted executable once when opening the adapter; retain the absolute path for pool replacement | Relative path executes successfully twice with idle pooling disabled, requiring fresh workers |
| P2: CLI discarded Bash failure streams and partial virtual-change diagnostics | Failed execution emits bounded, byte-safe JSON diagnostics to stderr, no receipt/transaction handle, and a nonzero exit status | Small and truncated binary stdout/stderr survive base64 round-trip; exit 7 stays 7; unsupported profile failure exits 1; partial virtual writes never reach the host |

The async join also now owns exception retrieval through `asyncio.wait`, avoiding an
abandoned shield future when native failure races repeated cancellation. Its dedicated
regression captures event-loop errors and observes none. Documentation distinguishes
protocol cancellation from merely abandoning a client-side await: without a delivered
notification, the server cannot infer that the request was cancelled. Once commit
entry wins, transport cancellation cannot promise rollback or delivery of the result.

New regression and verification logs use the `review-` prefix under
`target/bashkit-evidence/p7/`. These tests exercise real native operations and real
workers; fault gates only select deterministic race/crash boundaries. No live judge,
credentials or paid model was used.

## Implemented surfaces

- Rust: optional `vsh`/`vsh-runtime` feature `bash`, `Language`,
  `RunRequest::with_language`, `RuntimeConfig::with_bash`, `ExecutionOutput`,
  `BashResult` and request-scoped cooperative cancellation. Default requests remain
  Monty. A Bash-only runtime does not require starting a Monty worker.
- Python/PyO3: `BashConfig`, seven-field `BashLimits`, `Language.BASH`, binary
  `BashResult`, typed failure diagnostics and both request/source preview overloads.
  Blocking work releases the GIL; async hook/capability work runs off the event loop.
- Hooks/judges: the same policy, review, approval, stale and single-use commit gate;
  Python `RequestEvent.execution_context` schema 2 includes language/profile/completion/stream lengths.
  `judge.hook_handler`, `review_instructions`, hook scope defaults and JEV's 0.69
  threshold are unchanged. Deterministic model tests make no paid/network calls.
- Capability/MCP/CLI: one existing `vsh_run` surface with host-enabled language
  selection. Model arguments cannot enable Bash. Pending/rejected capability results
  withhold both streams and the guest result. MCP display output has bounded base64
  and explicit truncation flags; native evidence remains complete and unchanged.
- Packaging: the PEP 517 wrapper independently builds and stages matching Monty and
  Bash executables. The default Rust host graph does not link Bashkit/Tokio; the
  worker-only graph has no commit/store/runtime authority.

## Safety and compatibility

The worker runs exact-pinned Bashkit 0.18.2 with default features disabled. It has no
RealFs, host shell/process fallback, network/SSH, inherited secret environment or
persistent interpreter. The parent alone owns the VFS, shared filesystem gateway,
policy, budget and ordered evidence. Each successful execution receives a sealed
canonical diff; committing does not rerun its program.

Current identities are `vsh-bash-bounded-v4` and
`vsh-bash-worker/4 bashkit/0.18.2 vsh/0.5.0`. Resolved argv guards reject unsupported
flags and lossy `printf` conversions, numeric byte escapes and the upstream `-v`
format-offset bypass, including nested dispatch. Binary `cat`/ordinary copying and
raw streams are preserved. The shell's string-valued variables are not full POSIX
binary strings. See the [profile](../docs/integrations/bash.md).

V3 pending artifacts retain a closed output tag, raw bytes, successful completion,
profile, complete ordered evidence and exact identity. Unknown tags, malformed or
over-limit output, altered seals and incomplete executions fail closed. Old V1/V2
records are not re-signed. A nonzero/timeout/profile failure has diagnostics but no
commit handle; `changes_complete` distinguishes an empty diff from an unavailable
diagnostic diff after resource failure.

Cancellation is arbitrated before native commit entry. Cancellation joins execution
and cleanup; an unseen ephemeral preview is discarded and durable automatic approval
is revoked to pending review. It does not erase audit history or roll back an entered
commit. Entered commits finish and return their actual outcome. Monty pipe writes,
startup checks and reset use controlled deadlines too. Host loader callbacks remain
cooperative rather than forcibly interrupted.

## Verified allocation and work reductions

Fresh artifact encoding/counting seals evidence in one bounded pass; promotion and
loading still verify an existing seal. Ephemeral previews do not allocate a temporary
artifact just to count its size. Completed VFS execution consumes `into_evidence`
and moves read/write sets and the ordered effects buffer. It checks sticky evidence
failure before transfer. Durable receipt return also moves rather than clones its
output/full-detail diff. Required separate ephemeral/cache owners still clone; no
claim of a globally zero-copy runtime is made.

The immutable Monty adapter template is cached per runtime; each request clones it
and changes only its budget. This avoids reconstructing and sorting a default call
policy before replacing it with the configured policy. The worker request moves its
already-owned program string instead of copying it again. Fresh cache insertion uses
the already-sealed transaction identity instead of hashing the binding a second time.
Load, promotion and stale/commit verification remain unchanged. A new regression
checks custom policy equality, virtual-root mapping, request limits and identity
changes across both in-process and deferred adapter configurations.

Behavioral tests verify evidence equality/order, effect-buffer ownership transfer,
sticky-failure rejection, byte bounds, legacy identity, tamper rejection, denied and
stale commits, real worker cancellation, repeated Python cancellation and commit
entry races. Fresh review identified the four findings resolved above. The local
correction verdict does not substitute for the remaining performance/platform gates.

## Verification ledger

Final command logs, raw benchmark samples, distributions and archives are retained
under ignored `target/bashkit-evidence/p7/`. These are local execution results, not
hosted CI results or authorization to publish the unchanged 0.5.0 package version.

| Gate | Current evidence |
| --- | --- |
| Rust workspace tests | 282 tests passed in the fresh all-feature/all-target coverage run, including the four Rust regressions from review/completion |
| Rust coverage | Lines 83.98%, functions 76.82%, regions 84.32%; existing 79/70/81 floors passed. Stable LLVM reported no branch counters, so this is not Rust branch coverage. Only existing `(vsh-python\|vsh-worker)` exclusions apply; Bash is not excluded |
| Clippy / rustdoc | Workspace all-feature/all-target Clippy with `-D warnings` passed; all-feature rustdoc with `-D warnings` passed, excluding the duplicate `vsh-runtime` library documentation target |
| Python line/branch coverage | 175 tests passed against the rebuilt release native extension; 664 statements and 178 branches, 100% each |
| Ruff / type checks | Ruff check and format, basedpyright and ty passed, including the new examples and benchmark |
| Rust dependency audit | 285 locked packages; no vulnerabilities; one existing unused optional `atomic-polyfill` unmaintained warning, not hidden |
| Cargo deny | Fresh advisory database: advisories, bans, licenses and sources passed; no new advisory waiver |
| Downstream dependency resolution | Fresh independent consumer lock resolved 236 packages; native `vsh` import compiled and its separate dependency audit passed with the same optional unmaintained warning |
| Feature boundaries | Default SDK and Python parent exclude Bashkit/Tokio; isolated worker excludes runtime, commit, store, execution gateway and Monty authority. Isolated feature builds and metadata guard are part of CI |
| Native wheel / sdist | Pinned Maturin 1.15.0 checkout wheel and sdist built; detached sdist rebuilt a wheel; both wheels passed archive validation and isolated installed preview/commit smoke checks with worker environment overrides unset |
| Runnable examples | Binary-safe Python preview/commit and deterministic capability review passed with the explicit checkout worker; the compiled Rust Bash preview/commit example passed. The optional live judge example was type-checked, not executed against a paid model |
| Documentation | Strict build; 38 pages, 51 Python snippets and 3,970 local links checked with no errors; LLM exports regenerated |
| Hosted / platform matrix | No push or hosted run performed; Linux conformance and Windows Monty portability are not locally verified |

## Performance method and limitations

### Closure investigation

The follow-up began with only 172 MiB free on the APFS volume. Removed only the
rebuildable `target/debug` and `target/llvm-cov-target` caches, preserving release
executables, raw benchmark samples and coverage reports; free space rose to 2.1 GiB.
This is an environment confounder to investigate, not evidence that the previous
regression was imaginary. `completion-baseline-a` began before the release rebuild
finished; that exploratory pair is retained but is not gate evidence. Temporary
persist-stage diagnostics will separate encoding, blob persistence and record
persistence; those diagnostics were removed before the isolated counterbalanced series.
The 5% threshold and all workloads remain unchanged.

The 30-sample diagnostic run (`completion-profile.json`/`.log`, plus warmup)
measured the 5,050-entry artifact at 2,418,907 bytes: encoding median 2.847 ms,
blob persistence median 23.021 ms / p95 144.192 ms, transaction-log median 4.476 ms.
This locates observed variance in persistence, not proof of a specific kernel or
storage cause. The temporary instrumentation has been removed. The next gate series
is predeclared as baseline C, candidate D, candidate E, baseline F: 100 warm samples
per case in each run; no simultaneous build/test jobs. Retain all four complete runs,
pool 200 samples per side and check both median and p95 against the unchanged 5% limit.

C–F completed: large delete now +2.11% median / −1.00% p95, but the series still
fails the all-case gate (no-op, read and edit tails). The identical candidate had
no-op medians 0.347 ms in D and 0.174 ms in E. Keep both; do not select the favorable
run. This also exposed the harness's single-call warmup on sub-millisecond workloads.
The next controlled comparison uses the **same updated benchmark source** on archived
baseline `8550fac` and the candidate, warming each workload for 500 ms before collecting
100 samples. Both use default crate features and the same worker. Baseline builds in
its own Cargo target to avoid source-artifact mixing; no runtime/security semantics
or 5% acceptance threshold changes. Predeclared order: warm baseline A, candidate B,
candidate C, baseline D. Prior failed measurements remain part of the record.

Warm-series provenance (SHA-256):

- Identical benchmark source in both trees: `36c731e2f86f7600462b494c8d618cdcb8d76fcb3f803c834373a9bd05236c20`.
- `native-warm-baseline`: `ad0e98c3e3599e435deb85d1ba380f5076fa56dc9a640df1c2673a233a85e587`.
- `native-warm-candidate`: `2b78f798845bf79100060a965a0189f051ea80705df4ee16cdac378d4cca868f`.
- Shared Monty worker: `cf57f2ea3c37e05e8d1472cc4d7e51f7ddaf5931f5c53390b6070995961bcb76`.

Both preserved executables live under `target/bashkit-evidence/p7/`. The isolated
baseline target cache was removed after copying its executable; archived baseline
source and evidence remain. No instrumented diagnostic binary is used for this gate.

The complete warm A–D series still fails the all-case gate. All 200 samples per
side are pooled using `ceil((n - 1) * p)`; state and changed-path counts match in
all four runs. The large-delete median's +4.9995% is only a marginal pass, not a
robust safety margin. No samples were dropped.

| Warm A–D case | Baseline p50 / p95 ms | Candidate p50 / p95 ms | Delta p50 / p95 |
| --- | ---: | ---: | ---: |
| noop | 0.171625 / 0.177375 | 0.178708 / 0.200750 | +4.13% / +13.18% |
| read_10 | 0.911375 / 1.074333 | 0.910375 / 0.935834 | −0.11% / −12.89% |
| edit_20 | 1.862250 / 1.899542 | 1.860834 / 1.913208 | −0.08% / +0.72% |
| search_10k | 61.530708 / 62.560208 | 60.294250 / 61.089084 | −2.01% / −2.35% |
| vsh_glob_10k | 65.980041 / 66.785791 | 67.517542 / 70.257084 | +2.33% / +5.20% |
| rename_subtree_100 | 68.631750 / 72.527958 | 69.174417 / 72.825167 | +0.79% / +0.41% |
| delete_subtree_100 | 58.013166 / 61.039666 | 57.975709 / 61.059208 | −0.06% / +0.03% |
| vsh_remove_subtree_100 | 57.014542 / 63.145166 | 56.988583 / 59.992333 | −0.05% / −4.99% |
| massive_delete_5k | 139.136542 / 149.502000 | 146.092708 / 154.598708 | +5.00% / +3.41% |

A bounded transport optimization follows this failed result: read the four-byte
Monty event prefix into its complete buffer first, then read only the missing
suffix. Previously even an available complete prefix always required separate
one-byte and three-byte reads. EOF, interrupted/fragmented input, prefix/body
truncation, the pre-allocation hard cap and the pre-decode kind cap remain enforced.
A regression checks two concatenated frames across five fragmentation sizes,
alternating interruptions, exact frame consumption, all partial prefix lengths,
and rejection of an oversized prefix without consuming its body. The unfragmented
case requires two reads per event instead of three. This is an I/O-work reduction,
not yet a measured latency claim.

The header series was predeclared as `header-baseline-a`, `header-candidate-b`,
`header-candidate-c`, `header-baseline-d`: the same warm harness, 100 measured samples
per case, five cold samples and two independent runtimes. Save the new candidate as
`native-warm-candidate-header`; preserve both earlier executables and every prior
series. Start only after tests/builds finish; pool all four complete files and keep
the same nine workloads and 5% p50/p95 gate.

All four `header-` files completed. The candidate executable SHA-256 is
`2420c3be457cae4172ff1fc9202aa55175ad7203c55adf6f5bfef59a609c021f`;
baseline, harness and worker hashes remain as listed above. Every file has the same
environment/protocol fields and 100 samples for each of nine cases. All 400 samples
per case agree on state and changed-path count. This series **also fails**; do not
pool it with the older candidate or rerun until a favorable subset appears.

| Header A–D case | Baseline p50 / p95 ms | Candidate p50 / p95 ms | Delta p50 / p95 |
| --- | ---: | ---: | ---: |
| noop | 0.172916 / 0.184709 | 0.181125 / 0.350750 | +4.75% / +89.89% |
| read_10 | 0.912916 / 1.001792 | 1.000500 / 2.021625 | +9.59% / +101.80% |
| edit_20 | 1.868750 / 1.937334 | 2.075834 / 2.821958 | +11.08% / +45.66% |
| search_10k | 62.587208 / 67.135500 | 64.251583 / 92.317625 | +2.66% / +37.51% |
| vsh_glob_10k | 68.321167 / 91.531959 | 70.217917 / 84.097375 | +2.78% / −8.12% |
| rename_subtree_100 | 70.863292 / 74.554416 | 71.019791 / 99.029083 | +0.22% / +32.83% |
| delete_subtree_100 | 58.993417 / 68.009250 | 58.914208 / 62.146709 | −0.13% / −8.62% |
| vsh_remove_subtree_100 | 57.986083 / 61.223750 | 57.977792 / 65.076750 | −0.01% / +6.29% |
| massive_delete_5k | 144.306500 / 159.536000 | 150.740916 / 173.210000 | +4.46% / +8.57% |

No runtime security/evidence check, durability operation, workload or acceptance
threshold was removed. The source/test review approves the narrow prefix-read
change; it does not attribute these noisy end-to-end results to a specific cause or
claim a latency improvement. The full Monty release suite passes 36 tests, including
the new fragmented-frame regression. The original shared worker is additionally
preserved as `monty-worker-installed-checkpoint`, with the recorded `cf57f2…` hash;
subsequent package/feature builds may replace the ordinary release worker path.

Fresh final coverage (`completion-final-coverage.log` and
`completion-final-rust-coverage.json`) ran the complete workspace, all features and
all targets: **282 passed, zero failed**, with 17,555/20,903 lines,
1,485/1,933 functions and 27,636/32,774 regions covered. The same floors and filename
exclusions are unchanged. This includes the new transport test; there is no Rust
branch-coverage claim.

An additional `--release --workspace --all-features --all-targets` experiment failed
to link the Python extension's empty standalone Rust test executable: its
`extension-module` build expects Python symbols from the loading interpreter. The
binding crate contains no Rust unit tests. The normal complete-workspace coverage
gate above still passes, and no test or CI configuration was disabled. Release-core
verification excludes only that empty binding harness; actual binding behavior is
checked through the rebuilt extension and installed wheel under CPython. The failed
command log is retained as `completion-final-rust-tests.log`, not reported as green.
The release-core run passes the same **282 tests** (`completion-final-rust-core-tests.log`);
all-workspace/all-feature/all-target release Clippy also passes with `-D warnings`.
No source/configuration change was needed for that additional test-launch mismatch.

Final checkout packaging and Python validation pass: the rebuilt extension runs all
**175 Python tests**, 664/664 statements and 178/178 branches, with both worker
environment overrides unset. The wheel and sdist pass archive validation; the wheel
installs offline in a separate environment and its bundled Monty/Bash workers pass
real binary-safe preview/commit checks. No live model calls were used. Logs use the
`completion-final-` prefix. Ruff, format, ty and basedpyright remain clean.

The final source archive was extracted under
`/private/tmp/vsh-final-sdist.Ify3sj/vsh_python-0.5.0` and its own build backend produced
a second wheel. The extracted worker source, benchmark and backend match the checkout
byte-for-byte. This detached build reuses the existing Cargo target cache; it is not
a from-scratch hermetic-build claim. The second wheel/source archive pass validation,
and the installed second wheel passes the same real Monty/Bash binary preview/commit
smoke with worker overrides and `PYTHONPATH` unset. Raw logs and SHA256SUMS for both
sets remain under `completion-wheels` and `completion-sdist-wheels`.

CI preparation also found and fixed one test-only assumption: an explicit
`VSH_BASH_WORKER` override is optional, not mandatory. The CLI test now selects the
installed sibling executable when no override is supplied. With **both** worker
environment overrides unset, all 175 Python tests pass and line/branch coverage
remains 100% (664 statements, 178 branches). Ruff, format, ty and basedpyright pass.

The existing optimized native Monty harness uses the preserved baseline executable
and the candidate with identical disposable fixtures, policy, matching worker and
release profile. Each run retains 100 warm samples per workload, five cold-runtime
samples and two independent concurrent runtimes. The final counterbalanced
candidate/baseline/baseline/candidate series is `candidate-k2.json`,
`baseline-l.json`, `baseline-n.json`, `candidate-m.json`: 200 samples per side for
every workload. Quantiles use sorted pooled samples, index `ceil((n - 1) * p)`.
Earlier A–J measurements remain retained as intermediate checkpoints. The first K
run overlapped a test process and is excluded as perturbed; K2 is its isolated rerun.
Neither slow candidate run in the final series is discarded. No workload was removed
or security check bypassed to improve a number.

Local scheduling, worker warm-up, CPU state and durable I/O create visible spread,
especially sub-millisecond no-ops. A favorable pooled median is not proof of a causal
speedup. The final 5,000-file/50-directory deletion p50/p95 is
**145.581/152.873 ms baseline → 158.627/197.006 ms candidate**, or **+8.96%/+28.87%**.
All other final pooled cases remain within the proposed 5% median/tail target.
The complete all-case performance gate therefore remains **open**, not waived.

Large-delete candidate medians vary between 172.077 and 150.888 ms. In the slower
K2 candidate versus L baseline, bind/store stage medians were 47.686 versus 23.097 ms;
this stage includes encoding, sealing and durable I/O. Snapshot and execute medians
were 43.393/72.234 versus 43.707/70.385 ms. This is evidence to profile bind/store,
not proof of a specific CPU, worker-pipe or disk cause. The full workload table is in
[performance](../docs/performance.md).

`benchmarks/bash_runtime.py` independently measures the public PyO3 Bash surface on
a disposable 21-file fixture with a 1 MiB binary payload. Its defaults retain 50 warm
samples per case and 20 cold samples; raw output records state, changed paths, FS-call
counts and stage timings. Chmod previews include durable pending-review I/O, unlike
the auto-approved cases. `benchmarks/process_tree.py` measures tree RSS separately;
sampler-perturbed timings are excluded from latency comparisons. Summed process RSS
double-counts shared pages and is not an allocator peak or an isolation guarantee.

Final public Bash p50/p95: no-op **0.271/0.318 ms**, edit 20 files **1.927/2.084 ms**,
1 MiB pipeline **7.660/7.943 ms**, cold open/first preview **10.156/17.854 ms**.
The separate 53-sample RSS run observed **40.45 MiB parent** and **54.64 MiB summed
tree**, at most two processes. After review fixes, the checkout arm64 wheel is
**9,069,044 bytes**; the source archive is **358,485 bytes**. There is no comparable baseline wheel/RSS
measurement, so no percentage memory or package-size improvement is claimed.

## Remaining gates

- The all-case 5% performance gate is not satisfied. Latest controlled warm results
  fail read/edit medians and several tails, not only large deletion. All previous
  failed series remain retained. Use a clean controlled environment for subsequent
  attribution; do not weaken evidence, sealing, policy, durability or acceptance.
- Hosted Linux Bash conformance and Windows Monty portability have not run for this
  checkout. The existing workflows contain their checks; local macOS success cannot
  stand in for those results. The most recent green remote run at inspection time
  (`36762096862`) belongs to old HEAD `8550fac`, not the dirty candidate. An explicit
  question requesting permission to commit/push a separate review branch remains
  unanswered. After authorization, dispatch existing `ci.yml` on that exact branch:
  push alone does not run CI because its push trigger is restricted to `main`.
  Record the run URL, actual candidate SHA and Linux/Windows job conclusions. Do not
  merge to main or create a release as part of that authorization.
- Nothing was committed, pushed or released. New checkout APIs are not represented
  as already available in registry 0.5.0 artifacts. Release-readiness is not asserted.

## Reproduction

```sh
cargo build --release --locked -p vsh-monty-worker
cargo build --release --locked -p vsh-bash --no-default-features --features worker --bin vsh-bash-worker
export VSH_BASH_WORKER="$PWD/target/release/vsh-bash-worker"
uv build
uv run python examples/native/bash_workflow.py
uv run python benchmarks/bash_runtime.py --worker target/release/vsh-bash-worker --iterations 50 --cold-iterations 20 --output target/bashkit-evidence/p7/bash-latency.json
cargo llvm-cov --workspace --all-features --all-targets --locked --json --summary-only --output-path target/bashkit-evidence/p7/rust-coverage.json --ignore-filename-regex '(vsh-python|vsh-worker)' --fail-under-lines 79 --fail-under-functions 70 --fail-under-regions 81
```

Unix mode semantics are required by this initial Bash profile. Windows rejects Bash;
existing Monty support remains. No universal POSIX compatibility, streaming pipeline,
true parallel job, container-equivalence or multi-tenant sandbox guarantee is claimed.
