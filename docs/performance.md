# Performance and optimization evidence

## Stable integration decision — 2026-10-02

The latest local candidate also avoids an unsuccessful `mkdir` for every existing
blob shard. It inspects the entry without following links and creates only missing
directories. Opened/named identity checks, hash verification, locking and disk
synchronization are unchanged. A separate shared-path representation experiment
was rejected because it regressed reads and changed evidence-budget accounting.

This series compares the **pre-Bash reference** with the current integration,
not with the intermediate optimization checkout below. Both use the same worker,
release-profile harness and permissions on macOS arm64: 500 ms warm-up, 100 retained
samples per workload per run, ABBA ordering, 200 observations per side and
`ceil((n - 1) * p)` quantiles. Builds, tests and profilers ran separately.

| Workload | Reference p50 / p95, ms | Candidate p50 / p95, ms |
| --- | ---: | ---: |
| No-op | 0.143 / 0.167 | 0.138 / 0.151 |
| Read 10 files | 0.821 / 0.966 | 0.807 / 0.894 |
| Edit 20 files | 1.720 / 1.883 | 1.687 / 1.805 |
| Search 10K files | 49.528 / 57.786 | 43.103 / 47.061 |
| VSH glob 10K files | 58.187 / 73.602 | 52.800 / 56.197 |
| Rename subtree 100 | 56.984 / 81.198 | 55.170 / 71.938 |
| Delete subtree 100 | 46.982 / 67.767 | 43.967 / 55.095 |
| VSH remove subtree 100 | 44.216 / 58.995 | 42.009 / 45.923 |
| Delete 5K files | 134.083 / 155.528 | 129.465 / 164.458 |

All receipt states and changed-path counts agree. Eight workloads satisfy the 5%
p50/p95 target. Delete-5K improves p50 by 3.44%, but p95 regresses **5.74%**:
8.930 ms above the reference, or 1.154 ms beyond the allowed 5% boundary.
The project owner accepted **this measured performance exception** for integration
on 2026-10-02. It is not a passing benchmark, a general threshold increase, a
security/CI waiver, or a cross-platform performance guarantee. No release is implied.

Reference tails vary materially between series. These end-to-end comparisons do
not isolate the directory-opening change or prove it caused each improvement.
Earlier unfavorable runs remain below and in the evidence plan; unchanged code
was not repeatedly measured until a favorable result appeared. Raw runs and binary
identities are recorded in `target/stable-readiness-evidence/` and
`plans/policy_fix_and_latency_2026_10_02.md`. The Python/Bash table below predates
the last directory-opening change; it is not a new measurement of that change.

## Wildcard correction and canonical-diff work — 2026-10-02

This follow-up fixes protected wildcard matching for literal-star filenames and
binds the corrected semantics to policy v3. It also borrows canonical-diff candidate
paths, expands each deleted subtree once, avoids temporary node/origin clones,
short-circuits exact missing/tombstoned node lookups, and writes each durable state
frame in one call instead of three. Checksums, locks, `sync_all`, snapshot freshness,
evidence bounds and commit revalidation remain in place. Old pending authority
requires a [fresh preview under the current policy](security.md#policy-changes-invalidate-pending-authority).

The final native series compares the preceding optimized checkout with this
candidate. It retains 100 observations per workload per invocation, 500 ms warm-up,
and before/after/after/before ordering: 200 samples per side, using the ceil-index
quantiles described below. Both versions use the same worker and harness, outside
the filesystem sandbox. No builds, tests or profilers overlapped these runs.

| Workload | Before p50 / p95, ms | After p50 / p95, ms |
| --- | ---: | ---: |
| No-op | 0.139 / 0.158 | 0.139 / 0.155 |
| Read 10 files | 0.818 / 1.048 | 0.793 / 0.959 |
| Edit 20 files | 1.747 / 2.149 | 1.666 / 1.770 |
| Search 10K files | 43.863 / 49.429 | 43.246 / 51.042 |
| VSH glob 10K files | 50.527 / 55.151 | 50.290 / 58.673 |
| Rename subtree 100 | 53.922 / 58.338 | 53.967 / 63.354 |
| Delete subtree 100 | 43.071 / 47.997 | 43.011 / 46.932 |
| VSH remove subtree 100 | 42.050 / 46.201 | 43.093 / 52.906 |
| Delete 5K files | 128.000 / 145.404 | 129.942 / 150.507 |

The targeted delete-5K **diff stage** fell from per-run medians of 5.290/5.220 ms to
3.333/3.281 ms, approximately 37%. This is not a 37% reduction in total request
latency: its end-to-end median rose 1.52%. Read-10 and edit-20 medians improved
3.15% and 4.64%, but **the 5% all-case p95 gate still fails** for glob (+6.39%),
rename (+8.60%) and VSH remove-100 (+14.51%). Every receipt state and changed-path
count agrees. No release or merge-readiness claim follows from these measurements.

The final parent peak RSS observations were 33,783,808 / 32,653,312 bytes before and
31,948,800 / 32,096,256 after. This series is lower, but the earlier candidate's
ranges overlapped; there is no demonstrated uniform process-tree memory reduction.
Workers are excluded from these parent figures.

### Final Python/Bash series

The unchanged public harness uses the old/new release-profile native extensions
and one identical worker, again in ABBA order: 100 warm samples per side and 40
cold opens. Median and floor-index p95 are used. All state/path/call/byte counters
agree, and the new extension was restored after measurement.

| Workload | Before p50 / p95, ms | After p50 / p95, ms |
| --- | ---: | ---: |
| No-op | 0.186 / 0.239 | 0.187 / 0.237 |
| Read 10 files | 0.961 / 1.171 | 0.988 / 1.137 |
| Pipeline 1 MiB | 7.987 / 8.418 | 7.972 / 8.372 |
| Edit 20 files | 1.802 / 2.088 | 1.815 / 1.978 |
| Copy 1 MiB | 3.511 / 3.664 | 3.511 / 3.683 |
| Chmod 20 files | 13.003 / 14.110 | 12.946 / 15.366 |
| Cold open + first preview | 5.824 / 6.063 | 5.882 / 9.880 |

Chmod bind/store median is essentially unchanged: 11.884 → 11.842 ms. Fewer write
calls did not establish a durable-I/O speedup; its p95 and the cold-start p95 worsened.
The first complete candidate series also had unfavorable Bash tails and a native
remove-100 tail failure. It is retained, not replaced by an unchanged-code retry.
The second candidate changes point lookup; both series and exact binary identities
are documented in `plans/policy_fix_and_latency_2026_10_02.md`. Raw evidence is under
`target/policy-latency-evidence/`. Confirm tail latency on an isolated runner with
adequate free disk space before treating it as an acceptance pass. No durability
guarantee should be weakened to meet a latency target.

## Earlier core optimization checkout — 2026-10-02

This earlier local comparison starts from the already implemented Bash integration,
not from the pre-Bash release. It removes repeated path copies, builds snapshot
child indexes as sorted vectors, shares ancestor validation across directory
siblings, and compiles policy component shapes once. Authorization also extracts
the basename once per path, not once per rule. No dependency, public API, policy
threshold, snapshot freshness check or durability guarantee changed.

The native comparison uses the same release-profile harness and worker on both
sides: 500 ms warm-up per workload, 100 retained samples per run, ordered
before/after/after/before, pooling 200 observations per version. Quantiles select
`ceil((n - 1) * p)`. Builds, tests and profilers ran separately. Both versions ran
outside the filesystem sandbox because macOS process-memory accounting required it.
Absolute timings must not be mixed with earlier sandboxed measurements.

| Workload | Before p50 / p95, ms | After p50 / p95, ms | Median change |
| --- | ---: | ---: | ---: |
| No-op | 0.141 / 0.165 | 0.137 / 0.168 | -2.89% |
| Read 10 files | 0.822 / 1.004 | 0.800 / 0.893 | -2.69% |
| Edit 20 files | 1.706 / 2.000 | 1.677 / 1.861 | -1.69% |
| Search 10K files | 48.130 / 63.493 | 42.546 / 45.540 | -11.60% |
| VSH glob 10K files | 53.915 / 58.061 | 49.733 / 51.877 | -7.76% |
| Rename subtree 100 | 55.037 / 62.616 | 53.012 / 72.885 | -3.68% |
| Delete subtree 100 | 45.983 / 52.952 | 42.960 / 48.587 | -6.57% |
| VSH remove subtree 100 | 44.081 / 53.078 | 42.060 / 47.081 | -4.58% |
| Delete 5K files | 164.618 / 204.871 | 157.002 / 216.532 | -4.63% |

All receipt states and changed-path counts agree. All nine medians improved, but
**the 5% all-case tail gate still fails**: rename p95 rose 16.40%, delete-5K p95
rose 5.69%. Earlier unsuccessful optimization attempts and unfavorable runs were
retained. These are checkout measurements, not release or merge-readiness claims.

The policy microbenchmark's median dropped 28.94% (5.231 → 3.717 ms per 10K
authorizations); its p95 rose 22.12%. It uses three complete ABBA blocks with 60
samples per side and verifies exactly 1,000 denied paths per invocation. This
microbenchmark does not replace end-to-end measurements.

Parent-process peak RSS was 33,423,360 / 33,308,672 bytes before and
31,506,432 / 33,865,728 bytes after. These ranges overlap: there is **no demonstrated
uniform memory reduction**. The unchanged worker's memory is excluded. This shared
macOS arm64 machine also had less than 1 GiB free; durable-I/O tail behavior needs
confirmation on a dedicated runner. Raw data is under
`target/optimization-evidence/`; implementation decisions, rejected attempts,
correctness evidence and remaining costs are recorded in
`plans/performance_optimization_2026_10_02.md` in the checkout.

### Updated public Python/Bash comparison

The unchanged Bash harness also ran before/after/after/before using the old and
rebuilt native extension, with one identical worker. Each side pools 100 warm
samples per workload and 40 cold opens. Bash uses the median and the
`floor((n - 1) * 0.95)` tail index. State, changed-path count, OS calls and byte
accounting agree in every sample.

| Workload | Before p50 / p95, ms | After p50 / p95, ms |
| --- | ---: | ---: |
| No-op | 0.197 / 0.249 | 0.193 / 0.291 |
| Read 10 files | 1.096 / 1.393 | 1.109 / 2.276 |
| Pipeline 1 MiB | 8.093 / 10.351 | 8.049 / 8.586 |
| Edit 20 files | 2.104 / 3.832 | 1.987 / 2.243 |
| Copy 1 MiB | 3.772 / 4.391 | 3.581 / 4.359 |
| Chmod 20 files | 13.018 / 18.214 | 14.006 / 19.322 |
| Cold open + first preview | 9.401 / 10.907 | 9.520 / 11.602 |

Editing and copying medians improved by 5.55% and 5.07%. This does **not** imply
every Bash workload improved: chmod regressed 7.59% in median and 6.08% in p95,
and read/no-op tails worsened. Chmod's measured execution, snapshot and policy
stages decreased, while its bind/store median increased from 11.880 to 12.783 ms.
Required durable storage remains a substantial cost; no synchronization or
integrity check was removed to improve these numbers.

## Bash integration checkout — 2026-10-01–02

These are local macOS arm64 **checkout** measurements, not a published release or
cross-platform latency guarantee. Optimized native builds use exact Bashkit 0.18.2,
Monty 0.0.22 and matching supervised workers. All timed calls are previews, not commits.

### Public Python Bash surface

`benchmarks/bash_runtime.py` uses a disposable 21-file workspace, including one
1 MiB binary file. Each warm case retains 50 samples after one discarded warmup;
20 independent cold runtime/first-call samples measure startup separately. Each call
still captures a fresh snapshot; process reuse does not reuse interpreter state.
Bash p50 is the sample median; p95 selects sorted index `floor((n - 1) * 0.95)`.

| Workload | p50 / p95, ms | Policy outcome |
| --- | ---: | --- |
| No-op | 0.271 / 0.318 | Auto-approved |
| Read 10 files | 1.037 / 1.322 | Auto-approved |
| 1 MiB `cat` → `wc` pipeline | 7.660 / 7.943 | Auto-approved |
| Edit 20 files | 1.927 / 2.084 | Auto-approved |
| Copy a 1 MiB binary file | 3.585 / 3.641 | Auto-approved |
| Change modes on 20 files | 17.996 / 21.721 | Pending approval; includes durable I/O |
| Cold open + first Bash preview | 10.156 / 17.854 | Auto-approved |

A separate 20 ms process-tree sampler observed **40.45 MiB parent peak** and
**54.64 MiB summed tree peak**, with at most two processes, on this same small fixture.
The 53 samples can miss shorter peaks, and summed RSS double-counts shared pages.
Sampler-perturbed latency is excluded. This is not a memory ceiling, allocation-peak
measurement, or evidence that larger snapshots have the same footprint.

The final checkout CPython 3.14 macOS arm64 wheel is 9,069,044 bytes and bundles both workers.
No before/after wheel-size or RSS saving is claimed without a matching baseline.
The default Rust SDK and Python parent dependency graphs exclude Bashkit/Tokio;
only the independent Bash worker links them.

### Monty regression assessment

Preserved baseline and candidate native binaries used identical policies, fixtures,
release settings and the same worker. After the adapter-template/program-copy fixes,
an earlier counterbalanced candidate/baseline/baseline/candidate series retained 100
samples per run: 200 per side per workload.
Quantiles below use sorted pooled raw samples, index `ceil((n - 1) * p)`; pooling is
descriptive, not proof of a causal speedup. Earlier measurements remain retained too.

| Workload | Baseline p50 / p95, ms | Candidate p50 / p95, ms |
| --- | ---: | ---: |
| No-op | 0.179 / 0.228 | 0.173 / 0.206 |
| Read 10 files | 0.942 / 1.704 | 0.932 / 1.322 |
| Edit 20 files | 1.915 / 4.560 | 1.871 / 3.454 |
| Filter filenames in 10,000-file tree | 61.535 / 64.581 | 59.334 / 61.284 |
| Generic glob in 10,000-file tree | 66.229 / 70.684 | 66.652 / 68.730 |
| Rename 100-file subtree | 69.126 / 87.405 | 68.865 / 71.021 |
| Remove 100-file subtree | 57.978 / 63.377 | 57.955 / 59.052 |
| `vsh_remove` of 100-file subtree | 56.988 / 60.911 | 56.036 / 58.031 |
| Delete 5,000 files + 50 directories | 145.581 / 152.873 | 158.627 / 197.006 |

**The proposed 5% all-case median/tail non-regression gate is not closed.** That
pooled large-delete median rose 8.96% and p95 rose 28.87%. Its two candidate-run
medians were 172.077 and 150.888 ms: the slower run is retained, not discarded to
make the gate green. Bind/store time includes bounded encoding, sealing and durable
I/O; a broad CPU/pipe attribution is not established. Other pooled cases in that series remain
within the target, including the no-op regression identified in earlier runs.

#### Completion check: symmetric warm-up and bounded transport

Subsequent runs exposed nonstationary small-call timings. Both archived baseline
and candidate therefore received the same 500 ms per-workload warm-up, with no
changes to timed work, expected state, durability or the 5% gate. The first complete
warm comparison still failed no-op and glob p95. Those results remain retained.

The latest candidate also eliminates one redundant read of each available Monty
message prefix. Fragmented input, interrupted reads, exact frame boundaries and
pre-allocation/pre-decode limits remain tested. Fewer reads do **not** establish
lower end-to-end latency: the complete follow-up below still fails the gate.

The predeclared baseline/candidate/candidate/baseline order retains 100 samples per
run, pooled to 200 per side with the same ceiling-index quantiles. Both binaries use
default crate features, identical benchmark source and the same worker. No builds,
tests or RSS sampler ran alongside the series. This tests the default Rust Monty
path; it is not an all-feature or Python performance guarantee.

| Workload | Baseline p50 / p95, ms | Candidate p50 / p95, ms |
| --- | ---: | ---: |
| No-op | 0.173 / 0.185 | 0.181 / 0.351 |
| Read 10 files | 0.913 / 1.002 | 1.001 / 2.022 |
| Edit 20 files | 1.869 / 1.937 | 2.076 / 2.822 |
| Filter filenames in 10,000-file tree | 62.587 / 67.136 | 64.252 / 92.318 |
| Generic glob in 10,000-file tree | 68.321 / 91.532 | 70.218 / 84.097 |
| Rename 100-file subtree | 70.863 / 74.554 | 71.020 / 99.029 |
| Remove 100-file subtree | 58.993 / 68.009 | 58.914 / 62.147 |
| `vsh_remove` of 100-file subtree | 57.986 / 61.224 | 57.978 / 65.077 |
| Delete 5,000 files + 50 directories | 144.307 / 159.536 | 150.741 / 173.210 |

Read/edit medians and several tails exceed 5%; large deletion is +4.46% median and
+8.57% p95. Sample state and changed-path counts agree in all four runs. No slow run
was removed, and no CPU/storage cause or overall speedup is claimed. The machine
also experienced severe disk-space pressure during this investigation. Clean,
controlled follow-up measurements are required; a favorable subset is not approval.

The implementation removes evidence-set/effect-buffer cloning, redundant durable
receipt cloning, repeated default-policy construction, a second program-string copy,
redundant fresh cache-identity hashing and a second fresh artifact sealing pass.
These are ownership/work reductions with correctness tests, **not** a claim that the entire integration is
faster. Active evidence bounds, output sealing and cancellation remain enabled.
Further profiling and clean hosted/platform evidence are required before declaring
the complete performance/release gate passed. Raw measurements and command logs live
under ignored `target/bashkit-evidence/p7/`; the engineering summary is
`plans/bashkit_p7_p9_evidence.md` in the checkout.

## Historical optimization — VSH 0.4.0, 2026-09-05

The 2026-09-05 release-profile measurements show roughly **25–30% lower median latency**
for the large filename-discovery, glob and bulk-delete workloads after this optimization.
The strongest repeatable gain is less CPU and allocation work inside execution—not
skipping snapshots, policy, dependency checks or durability.

These are local macOS arm64 results for the **VSH 0.4.0 Monty 0.0.22 implementation**,
not cross-platform guarantees or service-level objectives.

## Measurement protocol

- Host: macOS 26.1, Apple arm64, 8 logical CPUs; CPython 3.14.6.
- Both the native harness and PyO3 extension use optimized release builds and a matching
  supervised worker. No in-process execution shortcut.
- Per run: 40 retained warm samples per case after one discarded warmup; 20 independent
  cold-runtime samples; four independent runtimes in the concurrency case.
- Large fixture: 100 directories with 100 files each. Small fixture: 20 input files.
- Baseline captured **before** runtime optimization. Final and independent confirmation
  runs use the same workloads, counts, decisions and limits.
- Timed cases are previews, including durable pending-approval storage where required;
  they do not measure actual application/commit latency.

Raw JSON, environment details, intermediate results and caveats live in
[`benchmarks/results/2026-09-05/`](https://github.com/fswair/vsh/tree/main/benchmarks/results/2026-09-05).
The generated `comparison.json` / `comparison.md` include every case, p95 values,
confirmation runs and stage distributions. An initial **debug-extension diagnostic**
is retained but explicitly excluded from the release comparison.

## Warm preview latency

Milliseconds, p50. Before → final; negative outcomes have not been removed.

| Workload | Rust before → final | Python before → final |
|---|---:|---:|
| No-op | 0.267 → 0.167 | 0.181 → 0.287 |
| Read 10 files | 0.869 → 0.887 | 0.861 → 1.024 |
| Edit 20 files | 1.856 → 1.943 | 1.840 → 1.774 |
| Filter names in 10,000-file tree | 67.922 → 49.002 | 68.130 → 48.689 |
| `vsh_glob` in 10,000-file tree | 71.975 → 54.002 | 73.716 → 51.964 |
| Rename 100-file subtree | 56.976 → 55.962 | 58.011 → 55.977 |
| Remove 100-file subtree with typed OS calls | 47.952 → 46.048 | 48.875 → 45.981 |
| `vsh_remove` of 100-file subtree | 48.012 → 45.082 | 47.801 → 45.057 |
| Remove 5,000 files + 50 directories | 175.077 → 131.162 | 182.400 → 130.868 |

The name-filter case knows the fixture has two directory levels and returns a count.
The glob case uses generic recursive matching and returns 1,000 typed paths. Neither
is a text-content search benchmark, and they are not identical result contracts.

### Confirmation and tail behavior

An independent repeat of the optimized binary measured:

| Workload | Rust repeat p50 / p95 | Python repeat p50 / p95 |
|---|---:|---:|
| Filename filtering | 50.677 / 52.319 | 49.045 / 51.466 |
| Generic glob | 57.591 / 63.819 | 51.775 / 52.185 |
| Bulk delete preview | 138.012 / 147.617 | 133.015 / 138.733 |

The large-workload gain repeats, though its magnitude varies. Small calls are sensitive
to scheduling and filesystem state: Python's no-op/read repeat was 0.153/0.886 ms,
while the first final run was 0.287/1.024 ms. The Rust confirmation's durable rename
case also rose to 66.967 ms. **No general small-call, durable-I/O or tail-latency
improvement is claimed.** These sequential local runs are not randomized A/B trials.

## Where time was removed

Rust p50 stage times, milliseconds:

| Stage within workload | Before | Final |
|---|---:|---:|
| Filename-filter execution | 34.893 | 17.429 |
| Glob execution | 36.445 | 19.765 |
| Bulk-delete execution | 101.734 | 69.890 |
| Bulk-delete canonical diff | 5.657 | 4.321 |
| Bulk-delete final policy evaluation | 11.600 | 3.534 |
| Filename-filter snapshot | 32.048 | 30.621 |

Call-policy checks performed during traversal are charged to **execute**, not just the
final `policy` stage. Stage medians are measured independently and need not sum to the
wall-time median. Durable binding/storage remains real work; it was not bypassed.

### Algorithms and data structures

| Change | Removed work | Preserved contract |
|---|---|---|
| Cursor-based policy globstar matcher | Per-path component vectors and one DP allocation per pattern component | Same matching language, canonical rules and first denial |
| Leading-globstar basename specialization | Scanning parent components for `**/*.key`-style rules | Same root and nested-path semantics |
| Canonical `VPath` fast path | Separator replacement/vector/join for already canonical inputs | Portable normalization and rejection priority |
| Borrowed ancestor lookups | Allocating a new owned path for every ancestor probe | Tombstone and non-directory shadowing |
| Empty-overlay resolution/listing | Parent probes, temporary tree and redundant visibility lookups | Immutable base visibility |
| Slash-bounded overlay range | Scanning every unrelated overlay path for a directory listing | Exact component boundaries and sorted results |
| Fused snapshot indexing / entry insertion | Duplicate parent derivation and B-tree searches | Parent validation order and snapshot identity |
| Flat canonical-diff after-state buffer | Extra keyed tree and cloned path keys | Complete lazy after-materialization before reading before-states |

The policy matcher uses constant auxiliary memory and no recursion. It performs at
most O(pattern components × path components) component matches; wildcard work inside
each component is separate. Overlay child discovery visits the relevant lexical
subtree, not an unrelated whole overlay, and retains no extra permanent child index.

A separate 10,000-path microbenchmark, with 1,000 expected denials and ten retained
samples, measured policy authorization median **17.875 → 5.806 ms (67.5% lower)** and
canonical parsing **1.119 → 0.712 ms (36.4% lower)**. It isolates these costs; it is not
an end-to-end latency claim or an allocation-profiler trace.

Correctness checks compare the new matcher against the original dynamic-programming
oracle over 94,501 combinations, including empty paths and multiple globstars, plus
deep paths and compiled fast paths. Portable-path oracle checks, prefix-sibling listing
tests and generated VFS operation sequences protect normalization and diff semantics.

## CPU and memory costs

Command-reported user + system time for the complete harness, including fixture,
cold and concurrency work, fell from **18.82 to 15.27 seconds** for Rust and
**18.93 to 15.25 seconds** for Python. This is about 19% lower reported CPU time for
that matrix, not an isolated per-transaction CPU measurement or a billing estimate.

Separate process-tree RSS sampling every 50 ms recorded:

| Surface | Root peak MiB before → final | Summed tree peak MiB before → final | Max observed processes |
|---|---:|---:|---:|
| Rust | 36.31 → 33.42 | 56.09 → 42.22 | 5 → 2 |
| Python | 53.86 → 57.81 | 77.78 → 97.28 | 5 → 7 |

**These samples do not establish a repeatable RSS reduction or regression.** The
sampler missed different short-lived overlaps; summing RSS also double-counts shared
pages and is not unique memory/PSS. Python's sampled root peak increased despite
lower temporary allocation work. Command-reported RSS high-water marks use another
scope and are retained separately in `command-rusage.json`.

The implementation eliminates specific temporary allocations; a universal resident
memory win has not been demonstrated. No model calls, token prices, storage retention
costs or monetary savings were measured.

## Cold startup and parallel work

The final run's 20 independent cold samples measured runtime-open p50 of 28.07 ms
(Rust) and 28.62 ms (Python), followed by first-call p50 of 4.72/4.79 ms. Keep cold
startup separate from warm preview figures. Reuse runtimes when the trusted
workspace/configuration is stable.

Four independent runtimes measured 3.27× native and 3.30× Python throughput speedup
over the harness's sequential phase. This demonstrates useful concurrency in this
workload, not guaranteed linear scaling or same-workspace commit throughput.

## Reproduce

Build the current release extension and matching worker using [development](development.md).
Run the two surfaces **sequentially**, without simultaneous tests, compilers or memory
samplers:

```bash
cargo build --release --locked -p vsh-runtime --example native_benchmark
target/release/examples/native_benchmark \
  --iterations 40 --cold-iterations 20 --parallel-workers 4 \
  --worker "$PWD/target/release/vsh-monty-worker" --output native-rust.json

VSH_MONTY_WORKER="$PWD/target/release/vsh-monty-worker" \
  uv run --no-sync python benchmarks/native_pyo3.py \
  --iterations 40 --cold-iterations 20 --parallel-workers 4 --output native-python.json
```

For separate memory instrumentation:

```bash
uv run --no-sync python benchmarks/process_tree.py --output memory.json -- \
  target/release/examples/native_benchmark \
  --iterations 40 --cold-iterations 20 --parallel-workers 4 \
  --worker "$PWD/target/release/vsh-monty-worker" --output instrumented.json
```

Do not use `instrumented.json` as latency evidence. Rebuild and verify release artifacts
before comparing; the first diagnostic in this session caught a debug extension that
would otherwise have produced a misleading speedup claim.

## Remaining costs and deliberate boundaries

Fresh metadata traversal still costs about 30–31 ms on this large fixture. Reducing
the trusted workspace root is an immediate application-level lever. A TTL snapshot
cache would weaken freshness and was not introduced. Bulk typed-call loops still pay
IPC; use bounded compound functions where their semantics fit. Pending approval and
commit still pay real durable I/O and integrity/revalidation costs.

Hosted OS/storage measurements, controlled steady-state process memory, commit/recovery
performance and actual agent-loop cost remain separate evidence work. See
[efficient usage](guides/efficient-usage.md) for practical tuning.
