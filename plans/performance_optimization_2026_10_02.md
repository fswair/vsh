# Performance optimization — 2026-10-02

## Outcome

Implemented and verified three narrow core optimizations. Final native local
medians improve 1.69–11.60% across all nine workloads; policy micro median improves
28.94%. Python/Bash edit and copy medians improve approximately 5%, but read/chmod
and several tails regress. **No all-workload performance-gate, memory-reduction,
merge-ready or release-ready claim.** Exact favorable and unfavorable results follow.

Verification: 286 Rust tests; 175 Python tests; Python line/branch coverage 100%;
Rust lines 84.17%, functions 77.08%, regions 84.54%. Full-feature Clippy, formatting,
Python static checks, dependency-boundary guard and strict documentation checks pass.
No commit, push, release, version/dependency change or paid model call was made.
Short Circuit's measurement-first and independent-review process caught root-order
and literal-star compatibility issues before acceptance; both were corrected.

Scope: speed and memory of the existing runtime, preserving policy, exact ordered
evidence, transaction identity, stale checks and durable commit/storage. No dependency,
API, language-profile, threshold, publish or push changes are authorized here.
Existing dirty/untracked Bash implementation and previous measurements are preserved.

## System design and invariants

The prior completion candidate is the before-version, not the older pre-Bash commit.
Its native executable and matching worker are preserved under
`target/optimization-evidence/`. The same 500 ms warm-up, fixtures, policies and
sample states apply before and after. Prior failed gate series remain valid history.

Current stage evidence: 10K-node snapshot p50 44–46 ms; large-delete execution ~74 ms;
read-10 execution ~0.79 ms; large-delete binding/persistence ~25 ms. Storage profiling
already distinguished durable-I/O variance from encoding time. An attempted macOS
sampling profile lacked process-inspection permission; its timings are diagnostic,
not acceptance evidence. No specific kernel/CPU cause is inferred from that failure.

First target: unnecessary allocation and ordering work in snapshot/path handling.
`VPath::join` currently allocates a combined string, then copies it again when parsing
already canonical input. Snapshot freezing creates an owned parent per node and a
B-tree set per directory despite scanning unique keys in globally sorted order.
Subtree collection then walks that index with cloned traversal state and re-sorts.

Keep canonical normalization and error priority unchanged; validate each parent
before constructing its child index. The immutable node map makes that validation
reusable for subsequent children. Children remain unique and sorted. Use existing
slash-bounded map traversal for subtrees, preserving root inclusion and prefix-sibling
exclusion. No snapshot reuse, omitted metadata capture or weaker race checks.

## Implementation and measurement

1. Share normalization over borrowed/owned string input; reuse the join buffer.
2. Build sorted child vectors from the already sorted unique node map, borrowing
   parent keys and validating each parent once. Replace subtree DFS/re-sort with the
   existing bounded ordered-map traversal.
3. Add normalization-equivalence and mixed/deep/prefix-sibling snapshot regressions;
   run focused types/VFS/host tests, then build the unchanged native harness.
4. Predeclared isolated before/after/after/before series: 100 samples per workload per
   run, five cold samples, two independent runtimes. Pool all 200 samples per side
   using `ceil((n - 1) * p)`; retain failures and check state/changed-path equality.
   No build/test/sampler overlaps timed runs. Report stage effects and all workloads.
5. Pursue further optimizations only with a concrete cost model and measurements.
   Run semantic/security regressions, coverage and final static checks. Use independent
   read-only judgment for nontrivial internal representation changes; main implements.

This is not a promise that every workload improves or that the historical 5% gate
will close. Allocation/work reductions and measured wall-time gains are distinct.

## Phase 1 evidence

Implemented owned-input normalization, one validated child vector per directory,
and slash-bounded subtree traversal. Independent review found that root `.` is not
the lexicographically first valid path (`!` and space sort before it). Root traversal
now uses the entire ordered node map; punctuation fixtures lock down this contract.
The reviewer approved the corrected narrow change. Focused types/VFS/commit tests:
86 passed. No semantic or filesystem-security check was removed.

All four declared runs are retained as `phase1-{before-a,after-b,after-c,before-d}.json`
under `target/optimization-evidence/`. Each side pools 200 observations per case.
All receipt states and changed-path counts agree across the four runs.

| Workload | Before p50 ms | After p50 ms | p50 change | p95 change |
| --- | ---: | ---: | ---: | ---: |
| noop | 0.177000 | 0.177750 | +0.42% | +5.41% |
| read 10 | 0.981084 | 0.963833 | -1.76% | -6.35% |
| edit 20 | 1.883542 | 1.941500 | +3.08% | +7.36% |
| search 10K | 61.574125 | 59.790333 | -2.90% | -2.25% |
| glob 10K | 68.133875 | 66.733875 | -2.05% | -3.22% |
| rename subtree 100 | 70.759417 | 68.778000 | -2.80% | -6.72% |
| delete subtree 100 | 58.976375 | 57.047833 | -3.27% | +0.20% |
| VSH remove subtree 100 | 57.984875 | 56.997333 | -1.70% | +5.90% |
| delete 5K | 169.202417 | 168.015959 | -0.70% | +12.17% |

Large-case snapshot medians moved from approximately 43.7–44.5 ms to 42.2–43.5 ms.
This is a modest reduction in snapshot work, **not an overall performance-gate pass**.
The unfavorable tail measurements remain part of the evidence.

## Further profiling

A separately sampled native process showed substantial filesystem syscall time.
These samples include fixture creation and cleanup, so they do not by themselves
identify snapshot bottlenecks. A symbolized diagnostic build is used to attribute
the calls before selecting further changes; diagnostic runs are not latency evidence.

## Phase 2 design

Symbolized samples attribute the `fcntl` waits largely to required `sync_all`, and
metadata work to the per-node snapshot scan. Neither is removed. VFS listing also
repeatedly resolves every child's ancestors: once for visibility, twice for the
canonical digest's length-prefix and payload passes. All children share a parent.

Factor exact-node resolution from ancestor visibility. A listing validates its
parent once, then resolves direct children without walking those same ancestors
again. Preserve ordered streaming merge, immutable expected-state digest input,
entry/evidence limits and fail-closed behavior. No persistent cache or new allocation.
Add full-resolution oracle tests for overlay replacements and ancestor whiteouts.
Measure the combined candidate with the same before/after/after/before protocol
against the preserved before-version; do not mix diagnostic profiles into timings.

Phase 2 validation: 127 focused tests passed (including 35 doctests). Independent
source review found no blocker and confirmed shared-parent and immutable-digest
preconditions; no missing mandatory test. Its report is retained separately.

The complete `phase2-{before-a,after-b,after-c,before-d}.json` series preserves all
samples and identical state/change counts. Combined p50/p95 changes versus before:

| Workload | p50 change | p95 change |
| --- | ---: | ---: |
| noop | +0.29% | -7.32% |
| read 10 | -1.00% | -62.91% |
| edit 20 | +15.42% | +284.27% |
| search 10K | -1.15% | +1.77% |
| glob 10K | -2.48% | -7.65% |
| rename subtree 100 | -0.86% | +12.82% |
| delete subtree 100 | +0.18% | +73.68% |
| VSH remove subtree 100 | -4.82% | -3.73% |
| delete 5K | -0.04% | -0.0002% |

Tail variance is substantial in this series, on both before and after runs. It is
retained rather than discarded as noise; this series does not pass a 5% gate.
The implementation removes redundant lookups, but these timings do not demonstrate
a uniform end-to-end improvement.

## Phase 3 design

The same profile shows call-policy matching as another CPU consumer. Default rules
mostly use literal components or one `*` (exact name, prefix, suffix, prefix+suffix).
Use exact comparison or length-bounded prefix/suffix comparison for those shapes;
retain the existing general matcher for multiple stars. Rule ordering, access masks,
policy encoding and first-denial identity stay unchanged. No decision cache is added.

Add an independent byte-level dynamic-programming oracle over generated empty,
literal, overlapping-prefix/suffix, Unicode and multi-star combinations. Preserve a
before binary of the existing policy microbenchmark. Run ten samples per invocation
in before/after/after/before order, retaining all raw samples and exact denial counts.
Then measure the combined native candidate against the original before-version with
the same full protocol. Microbenchmark improvement alone is not an end-to-end claim.

The initial per-call shape-scanning attempt was rejected: pooled authorize p50
5.334 → 6.844 ms per 10K paths (+28.32%), p95 13.336 → 11.263 ms (-15.55%). Raw
`policy-{before-a,after-b,after-c,before-d}.json` and both executables are retained.
The revised implementation compiles each component's shape once alongside its
existing string, with no additional per-component heap allocation. General
multi-star matching is unchanged. This adds a small fixed record-size cost per
compiled component, not an unbounded path-decision cache.

Revised micro protocol: the same unchanged benchmark executables run in three
complete before/after/after/before blocks (60 retained samples per side overall).
No selective retry or sample exclusion; all blocks count. The final native ABBA
series also records parent-process peak RSS via `/usr/bin/time -l` on every run;
the separate unchanged worker's RSS is not included in that number.

Compiled-shape-only micro result (`policy-compiled-*`, all three ABBA blocks):
authorize p50 5.259 → 5.062 ms (-3.75%), p95 5.603 → 8.396 ms (+49.84%). Even the
unchanged parse control's p95 rose 47.76%; keep these tails, not a gate pass.

Independent review caught a behavioral edge: filenames may contain literal `*`.
The existing general matcher can treat that byte literally before its wildcard
fallback. Conventional prefix/suffix matching would therefore change some existing
denials under the same policy digest. The final fast path falls back to the original
matcher for such positive candidates. Generated values now include `*`: 116,281
legacy-equivalence checks plus 41,261 independent wildcard-oracle checks for names
without `*`. Explicit public pattern regressions cover the review counterexamples.
This turn does not silently change wildcard semantics; a future correction needs
its own policy-version and migration/security decision.

The final candidate also extracts the basename once per authorization rather than
once per basename rule. Repeat the three-block micro protocol on this materially
different implementation and retain earlier rejected results.

Final micro result (`policy-final-*`, all three ABBA blocks, 60 samples per side):
authorize p50 **5.230875 → 3.717000 ms (-28.94%)** per 10K paths; p95
5.420458 → 6.619292 ms (+22.12%). The unchanged parse control is +1.19% p50 and
+76.66% p95. These tails remain reported, with no claim that the microbenchmark
passes a tail gate. Every invocation still denies exactly 1,000/10,000 paths.
All 14 policy tests pass. Independent source review accepted the literal-star
compatibility fallback and basename hoisting; no unresolved finding remains in
these optimization changes. The pre-existing wildcard issue is documented, not
silently fixed under the existing policy digest.

## Final native comparison

All four `final-{before-a,after-b,after-c,before-d}` runs completed with 100 retained
samples per workload per run. Pooled before/after sides each contain 200 samples.
Every sample agrees on state and changed-path count. Same worker, unchanged harness,
500 ms per-case warm-up, same release features and limits. No build/test/profile
process overlapped these runs. macOS `time` required process execution outside the
filesystem sandbox for `kern.clockrate`; **both versions used that same context**.
Do not compare absolute values across the earlier sandboxed and this final series.

| Workload | Before p50/p95 ms | After p50/p95 ms | p50 change | p95 change |
| --- | ---: | ---: | ---: | ---: |
| noop | 0.141375 / 0.165459 | 0.137292 / 0.168209 | -2.89% | +1.66% |
| read 10 | 0.822333 / 1.004167 | 0.800209 / 0.893083 | -2.69% | -11.06% |
| edit 20 | 1.706250 / 1.999666 | 1.677417 / 1.861125 | -1.69% | -6.93% |
| search 10K | 48.130333 / 63.492917 | 42.545541 / 45.540250 | -11.60% | -28.28% |
| glob 10K | 53.915250 / 58.061250 | 49.733333 / 51.877375 | -7.76% | -10.65% |
| rename subtree 100 | 55.037125 / 62.616208 | 53.011708 / 72.885375 | -3.68% | +16.40% |
| delete subtree 100 | 45.982750 / 52.952166 | 42.960042 / 48.586959 | -6.57% | -8.24% |
| VSH remove subtree 100 | 44.080833 / 53.078125 | 42.060167 / 47.080666 | -4.58% | -11.30% |
| delete 5K | 164.618291 / 204.871458 | 157.002125 / 216.531792 | -4.63% | +5.69% |

All nine medians improved in this final local comparison. **The all-case 5% tail
gate still fails** for subtree rename and delete-5K; neither these tails nor earlier
unfavorable series were discarded. This is not release/merge readiness evidence.

Parent peak RSS in bytes: before 33,423,360 / 33,308,672; after 31,506,432 /
33,865,728. Ranges overlap, and one after run exceeds both before runs: **no robust
RSS reduction claim**. Worker memory is excluded. Required snapshot metadata and
durable-I/O costs remain rather than being traded for weaker correctness.

## Python/Bash verification protocol

After native timing completes, preserve the current Python extension as the before
artifact, build the new release-profile binding, and compare the unchanged public
`benchmarks/bash_runtime.py` harness in before/after/after/before order. Each run
retains 50 samples per workload and 20 cold opens; every side pools 100 warm and
40 cold observations. Use the same Bash worker and fixture. No builds, tests or
RSS samplers run alongside timed processes. Restore the new extension before final
tests. Report Bash p50 as the median and p95 with the harness's floor-index rule,
distinct from the native benchmark's ceil-index rule.

All four Python/Bash runs completed. Every receipt's state, changed-path count,
OS-call count and read/write-byte count agrees across versions; the worker hash is
identical. The new extension was restored after the last before-run.

| Workload | Before p50/p95 ms | After p50/p95 ms | p50 change | p95 change |
| --- | ---: | ---: | ---: | ---: |
| noop | 0.197313 / 0.248959 | 0.193291 / 0.290709 | -2.04% | +16.77% |
| read 10 | 1.095584 / 1.393458 | 1.108813 / 2.276000 | +1.21% | +63.33% |
| pipeline 1 MiB | 8.093000 / 10.351083 | 8.048500 / 8.586250 | -0.55% | -17.05% |
| edit 20 | 2.103771 / 3.832041 | 1.987042 / 2.243083 | -5.55% | -41.47% |
| copy 1 MiB | 3.772000 / 4.390833 | 3.580646 / 4.358834 | -5.07% | -0.73% |
| chmod 20 | 13.018313 / 18.214000 | 14.006355 / 19.322041 | +7.59% | +6.08% |

The Bash surface is not uniformly faster: read median and several tails worsened,
and chmod's pending-review path regressed in both median and tail. These valid
results remain visible; do not extrapolate native search gains to every Bash command.

Bash cold open + first preview (40 samples per side): median 9.401230 → 9.519521 ms,
p95 10.907167 → 11.601500 ms. In chmod, pooled stage medians show execution
1.009375 → 0.867271 ms, snapshot 0.224792 → 0.194292 ms and policy
0.014792 → 0.006521 ms; bind/store rises 11.879792 → 12.782751 ms. This identifies
the measured storage-stage cost without asserting that OS variance alone caused it.

Native delete-5K diff-stage per-run medians also rose from 4.334–4.378 ms to
5.241–5.428 ms, despite the lower total median. This residual stage regression is
not hidden by the aggregate improvement; further targeted profiling is warranted.

The direct macOS `cargo build -p vsh-python` attempt lacked Python extension linker
flags and failed on unresolved `_Py*` symbols. Rebuilding the library with Cargo's
root-target `-C link-arg=-undefined -C link-arg=dynamic_lookup` succeeded; this is
the macOS extension-linking configuration, not a source/API change.

## Boundaries and unresolved costs

- A fresh host metadata scan remains O(nodes) per transaction; no stale snapshot
  cache or watcher-based trust shortcut was introduced.
- Pending transaction and blob durability still uses the existing filesystem
  synchronization and content verification. Tail latency remains storage-sensitive.
- No thread pool, new dependency, public execution API or policy threshold was added.
- Snapshot child indexing now uses one ordered vector per nonempty directory instead
  of a tree set, and canonical joins reuse their owned string buffer. These remove
  work/allocations; process RSS is measured separately, not inferred from them.
- This host has less than 1 GiB free during the final series. The device/OS is shared
  and not a dedicated benchmark runner. Results are local observations, not a hosted
  CI or cross-platform performance guarantee. Unfavorable valid runs are retained.

## Final verification and handoff

All logs and raw reports are under `target/optimization-evidence/` (ignored build
artifacts). Only production edits in this optimization turn are path normalization
in `vsh-types`, snapshot/listing internals in `vsh-vfs`, and compiled component
matching in `vsh-policy`, plus their behavioral tests. Existing Bash integration
changes were preserved. Documentation and generated Markdown exports are updated.

| Check | Result |
| --- | --- |
| Rust workspace, all features and targets, LLVM coverage | 286 passed, zero failures |
| Rust lines | 17,795 / 21,142 = 84.168953%, floor 79% |
| Rust functions | 1,507 / 1,955 = 77.084399%, floor 70% |
| Rust regions | 28,066 / 33,197 = 84.543784%, floor 81% |
| Python/PyO3 runtime, CLI, MCP, capability and judge tests | 175 passed |
| Python line/branch coverage | 664 / 664 statements, 178 / 178 branches |
| Release Clippy, workspace/all-features/all-targets | Passed with `-D warnings` |
| Rustfmt and patch whitespace | Passed |
| Ruff check and format, ty, basedpyright | Passed |
| Default SDK, Python parent, Bash worker feature boundaries | Passed |
| Zensical strict build and docs validation | 38 pages, 51 Python snippets, 3,976 local links; no errors |

Coverage retains the existing `(vsh-python|vsh-worker)` reporting exclusion and
unchanged 79/70/81 floors; it does not claim Rust branch coverage or exclude Bash
code. On macOS the local coverage build uses the Python extension dynamic-lookup
linker flags. No live judge credentials or external model calls were used.

One Clippy finding required documenting the existing non-root-parent invariant's
panic condition in the snapshot builder; this post-measurement edit is doc-only.
No timed workload or acceptance threshold changed. Current Python extension hash
matches the measured final binding after restoring it from the last before-run.

To make room for coverage, only the validated, generated
`/Users/mert/Desktop/vsh/target/release/deps` Cargo dependency cache was deleted
(approximately 2.9 GiB). It can be regenerated by Cargo. Source, release binaries,
before/after benchmark executables, workers, raw measurements and reports remain.

### Important follow-ups

1. Reproduce the remaining storage-sensitive p95 regressions on a dedicated runner
   with adequate free disk. Keep the original pre-Bash 5% gate and all nine workloads;
   the local before/after improvement does not close that historical gate.
2. Profile delete-5K canonical-diff work separately: its stage median increased
   despite the total median improvement. Avoid making allocation/CPU claims from
   wall-time variance alone.
3. Correct the **pre-existing literal-star wildcard semantics defect** as separately
   scoped policy/security work, with transaction-identity/versioning implications
   explicitly handled. Examples `*` against `**` and `*.key` against `*.key.key`
   remain legacy behavior in this optimization. This is not an endorsement of that
   behavior; do not mistake equivalence testing for a security fix.
4. Hosted cross-platform CI and any packaging/publishing action remain outside this
   local optimization turn. Nothing was pushed or released.

### Artifact identity

| Artifact | SHA-256 |
| --- | --- |
| Native before | `2420c3be457cae4172ff1fc9202aa55175ad7203c55adf6f5bfef59a609c021f` |
| Native final | `82871b1d80f179e1c6db9b6f4efc38d54f2fdf7b9cf2bd0afee7ac76e7e09368` |
| Shared Monty worker | `cf57f2ea3c37e05e8d1472cc4d7e51f7ddaf5931f5c53390b6070995961bcb76` |
| Python before | `75b4b9cf2bbd28b6f5768d7fbc97b2b2e91063b89d52578346c8817986348974` |
| Python final | `95ab3706e0ffc28b9e8142e33d88f11d315e12706278f5e6a0706ba9458c1e96` |
| Shared Bash worker | `dbf400ca0eebad81f1c359eb3925136ada9373e4338778067538ab8aee4f5fc5` |
| Unchanged native harness | `36c731e2f86f7600462b494c8d618cdcb8d76fcb3f803c834373a9bd05236c20` |
| Unchanged policy harness | `ff91da9c4ad49f1b5767c8eea55de7144eafd7bae07fe9907b26e860fd9bfe11` |
