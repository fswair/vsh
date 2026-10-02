# Wildcard policy correction and remaining latency work — 2026-10-02

Latest target correction: the user clarified that `stable` was a naming mistake
and requested `main`. The reviewed integration commits are fast-forwarded to main;
push main without a PR or force-push. The existing main CI and documentation Pages
workflows apply normally. No package release, tag or deletion of the temporary
stable branch is requested. Earlier stable references below record the actual
sequence of decisions, not the final destination. The narrowly accepted performance
exception remains unchanged.

User authorization: fix the discovered literal-star policy defect and improve the
remaining performance problems. Preserve the existing dirty worktree and prior raw
measurements. No push, release, dependency update or paid/network model invocation.

### Follow-up authorization and design

The subsequent user request authorizes fixing failed scenarios and integrating
directly into `stable`, without a PR. This supersedes the no-push restriction above;
release/package publication remains out of scope. Remote inspection on 2026-10-02
found only `main` at `8550fac`; create `stable` from the reviewed candidate only
after the local gates, then run hosted CI on that exact candidate.

The final prior series failed glob, rename and recursive-remove p95. Per-run stage
distributions show the common 10K-node snapshot costs 28–30 ms at the median;
durable bind/store also has variable tails. The earlier sampler identifies real
metadata syscalls plus manifest construction. Do not cache host metadata across
transactions or remove syncs. A bounded candidate shares immutable `VPath` text
with `Arc<str>` instead of reallocating it in snapshot indexes, lazy loaders,
read/write sets and evidence. Normalization, ordering, hashing and wire bytes must
remain identical. This is a general ownership improvement, not a workload shortcut.
Fresh paths may cost more to construct; retain it only after measured comparison.

Implementation: change the private path representation; test clone independence,
borrowed ordered/hashed lookup, canonical bytes and the existing exhaustive path
oracle. Run types/VFS/commit/runtime tests and fresh judgment. Preserve the current
binary as the before candidate in `target/stable-readiness-evidence/`; then run
the unchanged nine-case native ABBA series (100 samples, 500 ms warm-up, five cold,
two independent runtimes). Preserve every run. Compare to both the immediate
pre-change executable and the retained historical pre-Bash executable. No 5% gate,
workload, evidence limit or durability exception is authorized.

The shared-text candidate was rejected, not promoted: the complete ABBA series
in `target/stable-readiness-evidence/native-{before-A,after-B,after-C,before-D}`
showed read_10 p50/p95 regressions of 6.14%/6.95%. Large delete improved 3.37%/7.98%,
but mixed results do not satisfy all-case acceptance. Independent review additionally
found changed representation-dependent evidence charging. Reverted only this turn's
path experiment; all earlier authorized security/performance work remains intact.
Focused candidate tests passed (90), but are not acceptance of the reverted change.

Next is the already-required historical comparison, not a retry of the failed
incremental comparison: retained pre-Bash `p7/native-warm-baseline` (SHA-256
`ad0e98c3e3599e435deb85d1ba380f5076fa56dc9a640df1c2673a233a85e587`) versus
this turn's `native-before` (SHA-256
`ee8f099fc5c35fbd027cb0181e1b4cc68e2adee17800ef5e29cbe8199a638651`), which
again corresponds to the restored production source. Predeclared order historical
A, current B, current C, historical D; same worker, harness warm-up, iterations,
permissions and nine-case 5% gate. Keep prior incremental tail failures visible;
historical success must not be described as every prior experiment passing.

Historical ABBA completed: 8/9 cases satisfy the 5% p50/p95 limit; rename p95 is
58.044209 -> 62.251125 ms (+7.25%). All states and changed-path counts match.
No merge acceptance yet. Bind/store tails, rather than diff median, account for
much of the rename tail. Inspecting the immutable store found one general repeated
syscall: each existing blob shard is first passed to mkdir, even when it already
exists. Change `open_or_create_real_dir` to inspect with no-follow metadata first,
creating only on NotFound; on creation races keep AlreadyExists handling and reread
metadata. Preserve the open-handle/named-entry identity verification and every
sync/hash/lock. This removes a failed mkdir per existing-shard open, not a security
check. Reject non-directory/symlink entries before creation as before. Test existing
directory reuse, creation, non-directory rejection and symlink rejection plus the
existing store race/corruption tests. Measure this distinct candidate against the
same retained pre-Bash baseline in a new ABBA series; keep both failed series.

### Existing-directory candidate result and remaining decision

Decision update: the user explicitly accepted the proposed narrow performance
exception and requested integration ("mergele o zaman doküman güncel mi?").
Proceed directly to `stable`, without a PR or package release. The 5% target stays
unchanged; document this failed measurement as an exception, not a pass. Update
performance/coverage/Bash documentation, regenerate LLM/copy bundles, run strict
docs and normal quality checks, commit the reviewed integration, push stable
non-forcibly and dispatch CI for that exact branch candidate. Keep main and Pages
deployment settings unchanged. Hosted CI failures remain actionable, not waived.

Implemented the no-follow inspect-first directory path, with missing/create-race
handling and all original opened/named identity checks. Added real reuse/non-file
and eight-caller concurrent creation tests. Store/commit release tests: 65 passed.
Fresh review `target/stable-readiness-evidence/store-review.md` found no new scoped
security/durability blocker. The concurrency test is not exhaustive fault injection.

Measured executable SHA-256:
`fff6140235c045c0f2798bfd2a35cb77cedc74a2ca504a02221ce1333b11289c`.
Raw series: `target/stable-readiness-evidence/native-store-{baseline-A,candidate-B,candidate-C,baseline-D}`.
Each side pools 200 samples/case; unchanged ceil-index p50/p95. Every state and
changed-path count agrees. Values below are wall milliseconds, baseline -> candidate.

| Case | Baseline p50 / p95 | Candidate p50 / p95 | Change p50 / p95 |
| --- | ---: | ---: | ---: |
| noop | 0.143167 / 0.167375 | 0.137541 / 0.150792 | -3.93% / -9.91% |
| read_10 | 0.820834 / 0.965792 | 0.806792 / 0.893500 | -1.71% / -7.49% |
| edit_20 | 1.719583 / 1.882791 | 1.687125 / 1.805416 | -1.89% / -4.11% |
| search_10k | 49.527750 / 57.785667 | 43.103333 / 47.061375 | -12.97% / -18.56% |
| vsh_glob_10k | 58.186709 / 73.601584 | 52.799542 / 56.197375 | -9.26% / -23.65% |
| rename_subtree_100 | 56.983625 / 81.198041 | 55.169917 / 71.938209 | -3.18% / -11.40% |
| delete_subtree_100 | 46.982334 / 67.766958 | 43.966834 / 55.095333 | -6.42% / -18.70% |
| vsh_remove_subtree_100 | 44.216000 / 58.995084 | 42.009417 / 45.923083 | -4.99% / -22.16% |
| massive_delete_5k | 134.083458 / 155.528042 | 129.464500 / 164.458083 | -3.44% / +5.74% |

The 5% all-case gate STILL FAILS: massive-delete p95 exceeds the allowed
163.3044441 ms by 1.1536389 ms. This is not a pass rounded to 5%. The reference's
tails also vary materially between series. These end-to-end numbers do not isolate
the mkdir change or prove that it caused all observed improvements. Do not rerun
unchanged code until a favorable series appears. No previous failure is discarded.

Main asked the user whether to retain the 5% blocker or explicitly accept this
documented performance exception, conditional on other quality gates. Until that
choice is received, no branch push/merge, CI dispatch or release is performed.
Final local verification of the store candidate completed: 295 Rust tests;
line coverage 18,219/21,558 (84.51155%), functions 1,529/1,977 (77.33940%),
regions 28,905/34,032 (84.93477%). Existing 79/70/81 floors and exclusions are
unchanged; stable LLVM does not report Rust branch coverage here. Release Clippy
all-workspace/all-features/all-targets with warnings denied, formatting and diff
checks pass. Rebuilt and installed the current PyO3 binding; all 182 selected
Python release-surface tests pass with 664 statements and 178 branches at 100%.
Logs and coverage JSON are in `target/stable-readiness-evidence/`.
A clean hosted candidate run is still required for platform evidence after any
authorized integration. No commit, push, PR, branch creation or release performed.

### Hosted integration follow-up

After explicit user approval, commit `37806ca465ffdfbe34e39ee0b2efdcc0956b21be`
was pushed directly to `stable`; no PR/release or main update. CI run 37026031937
exposed Windows gateway fixture cleanup failures followed by process abort. The
fixture's Drop removed its directory while its own BaseSnapshot still held pinned
blob-store directory handles. Release that owned snapshot before removal; retain
the cleanup assertion and add an explicit release-before-remove regression test.
Windows uses uncaptured test output so an abort cannot hide earlier failure details.
This is test-fixture lifetime correction, not removal of production handle pinning.

Linux tests completed, but llvm-profdata failed on a corrupt raw instrumentation
profile. Preserve that failed log. Do not lower floors, expand source exclusions or
silently ignore corrupt profiles. Run the corrected candidate through CI again;
if profiling corruption repeats, investigate its producer rather than declaring
the coverage check satisfied. All previous local acceptance evidence remains dated
to its exact tested source. A hosted pass is still pending.

Run 37027110708 verifies the fixture lifetime fix: the new cleanup regression and
32 gateway tests pass on Windows. Four remaining assertions incorrectly require
Unix-only creation modes (0700/0755 directories and 0644 new files), while the
existing Windows VFS contract normalizes writable directories/files to 0777/0666.
Use explicit platform expectations for those four assertions; do not skip tests
or change production permission behavior. Existing synthetic-node mode preservation
assertions remain unchanged. Linux coverage and Clippy passed on this run without
loosening profile merge behavior, floors or exclusions; the prior invalid-profile
failure remains recorded. Local full coverage after the fixture test: 296 tests,
84.50691% lines, 77.33940% functions, 84.93183% regions.

## System design

Security first: `*` in a policy component always represents the wildcard, including
when the filename itself contains `*`. Literal and compiled one-star components and
the general multi-star matcher must agree with an independent byte-DP oracle.
Remove the previous compatibility fallback rather than preserving the defect.

This changes policy semantics. Advance the policy schema/digest domain from v2 to
v3, and validate the active policy identity on approval, commit preparation, hook
resolution and direct commit before any authorization/state transition or host
mutation. Old artifacts remain inspectable; they require a fresh preview before
approval/commit. Recovery of already-entered durable commit journals must retain
its existing crash-consistency semantics, not become blocked halfway by a new policy.

Canonical diff currently clones paths into two tree sets and repeatedly expands
nested tombstones. Use borrowed candidate keys and expand only disjoint deleted
subtrees, preserving exact candidate/expanded counts, canonical path ordering,
before/after states and two-pass lazy materialization. Prefix siblings and root
punctuation ordering must remain correct. No evidence limit may be relaxed.

Bind/store investigation must separate encoding/metadata work from actual durable
sync latency. Remove redundant work only with a correctness argument. Do not remove
`sync_all`, post-install content verification, checksums, locks, stale checks or
crash-recovery boundaries to make a benchmark pass. A new persistent format or
weaker durability would require a separate design decision, not a silent shortcut.

Persistence inspection confirms that the state log writes each bounded frame in
three calls (length, payload, checksum). Assemble the identical frame in one bounded
buffer and issue one `write_all`; retain the existing checksum, size guard, exclusive
lock, append sync and compaction sync. This removes two writes per frame without a
new format or batching transactions. Verify exact bytes for optional artifact and
approval fields and retain torn-tail/corruption/reopen tests. The previously sampled
`sync_all` cost remains; do not promise a proportionate wall-time improvement.

## Implementation phases

1. Correct matcher and all literal-star regressions; bind corrected semantics to
   a new policy digest. Test direct and hook-based stale-policy rejection, including
   persisted artifacts after reopening, without consuming approvals or reservations.
2. Optimize canonical-diff candidate construction and reference resolution. Compare
   entries, digest and metrics to an independent full-scan expansion oracle over
   nested deletes, recreated nodes, rename overlap and prefix siblings.
3. Inspect and profile persistence CPU/syscall work. Implement a bounded improvement
   only when durability/security invariants stay intact; document residual I/O cost.
4. Fresh read-only security/performance judgment under Short Circuit. Main implements
   and triages. Run focused tests, then workspace coverage, Clippy, Python binding and
   documentation gates. Keep current coverage floors and exclusions unchanged.

## Measurement contract

Before executable: previous turn's preserved `target/optimization-evidence/native-final`.
Before Python binding: `target/optimization-evidence/python-native-final.so`.
New evidence directory: `target/policy-latency-evidence/`.

Final native comparison: same nine workloads, unchanged harness and worker, 500 ms
warm-up, 100 retained observations per case per invocation, five cold observations,
two independent runtimes. Execute before/after/after/before without simultaneous
builds, tests or samplers; pool all 200 observations per side, ceil-index quantiles.
Use the same process permission context for both versions. Preserve all valid runs.
Public Python/Bash comparison uses the unchanged harness, 50 retained samples/case,
20 cold observations per invocation, ABBA, median/floor-index quantiles.

Diagnostics and microbenchmarks are separate from acceptance. Do not infer a tail
gate pass from median gains or a memory reduction from fewer source-level clones.
The original pre-Bash regression gate is a separate historical comparison and its
prior failures remain recorded.

## Implementation and correctness evidence

- The general matcher handles wildcard syntax before literal equality. The compiled
  one-star fast path no longer falls back to the defective legacy matcher. Both
  implementations match the independent byte-DP oracle on all 116,281 cases,
  including UTF-8 and literal stars in filenames. Public capability tests cover
  every access kind and nested secret paths; Python exercises both supervised
  frontends without disclosing fixture content or committing the unrelated write.
- `vsh-policy-v3` changes the digest domain. `Runtime::approve`, `prepare_commit`,
  `resolve_commit_cancellable`, `commit_exact` and evidence regeneration validate
  the active digest. Rust reports `PolicyChanged`; Python maps it to the existing
  `VshStateError`. The policy remains immutable for the life of one runtime.
- Reopened auto-approved, pending and approved artifacts fail before state changes
  under another policy. Ready and Review preparations both fail. Records remain
  readable, and the original policy can still commit the untouched transaction.
  A fresh preview under the new policy obtains a new identity and can be approved.
- Fault injection at `OperationApplied(0)` and `CommitMarkerSynced` proves that a
  changed policy does not block rollback or roll-forward of an entered journal.
- Canonical-diff candidates borrow overlay/base keys. A tombstone already present
  in the expanded union skips its whole subtree. Final entries alone own path
  copies; reference resolution avoids temporary Arc/origin clones. Two-pass lazy
  after-state capture is retained. Full-scan union tests check entries, digest and
  all metrics over prefix siblings, nested deletes, recreation and rename; lazy
  rename is checked in both lexical directions with one capture and stable repeats.
- State frames keep the same single allocation but reserve room for framing too:
  prefix, payload and checksum now reach `write_all` together. This reduces the
  usual write calls from three to one, not the number of durable synchronizations.
  Exact wire bytes and pre-write size failure are checked for all optional fields.

Independent read-only judgment found no production blocker. Its two coverage
requests (entered-journal recovery and complete lazy-diff metrics) were implemented.
Its follow-up caught a Windows fixture issue: actual literal-star host filenames
are now explicitly POSIX-only; platform-independent Rust matcher cases still run.

Verification: 292 workspace/all-feature/all-target Rust tests passed. Coverage is
18,131/21,468 lines (84.46%), 1,519/1,967 functions (77.22%) and 28,729/33,852 regions
(84.87%). Existing floors/exclusions are unchanged. The rebuilt Python binding
passed 182 tests, 664/664 statements and 178/178 branches (100% each). Release-profile
workspace Clippy with warnings denied, fmt, Ruff check/format, CI-scoped ty and
basedpyright passed. No Rust branch-coverage claim is made.

The first broad Clippy run hit local disk exhaustion; two Python fixture subprocesses
also failed during that run. Those logs remain. After removing only regenerable
`target/debug/deps` and `target/llvm-cov-target/debug/deps` caches, the isolated
fixture check passed (zero model calls, zero commits, fixture files unchanged), then
the complete Python suite passed. Missing Cargo cache entries were fetched under
the unchanged lockfile; single-job Clippy passed. No source, measurement or lockfile
was removed. These failures are not performance measurements.

## Reproduction identities

SHA-256, measured binaries retained locally:

| Artifact | SHA-256 |
| --- | --- |
| Before native (`optimization-evidence/native-final`) | `82871b1d80f179e1c6db9b6f4efc38d54f2fdf7b9cf2bd0afee7ac76e7e09368` |
| After native (`policy-latency-evidence/native-after`) | `d9e5712d9c34d9bb2c5bf4d42961002e438a01c7f122b020ba52e69cf34daa3a` |
| Shared Monty worker | `cf57f2ea3c37e05e8d1472cc4d7e51f7ddaf5931f5c53390b6070995961bcb76` |
| Before Python extension | `95ab3706e0ffc28b9e8142e33d88f11d315e12706278f5e6a0706ba9458c1e96` |
| After Python extension | `6d28c1570b0af1b8bd165b320e1254cdc3d5c46814d80359be25f824a19c2ed4` |
| Shared Bash worker | `dbf400ca0eebad81f1c359eb3925136ada9373e4338778067538ab8aee4f5fc5` |
| Native benchmark source | `36c731e2f86f7600462b494c8d618cdcb8d76fcb3f803c834373a9bd05236c20` |
| Bash benchmark source | `ef8e8145c2c0f101449d50bceed21ff0203318d80f28fc3b638e83960a1aafbe` |

## First measured candidate and follow-up design

The complete native ABBA series (`native-before-A`, `native-after-B`,
`native-after-C`, `native-before-D`) is retained. Delete-5K canonical diff medians
fell from 5.230/5.226 ms to 3.150/3.152 ms, about 40%. End-to-end median was
128.006 → 127.575 ms, p95 166.565 → 140.970 ms. All states and path counts agree.
The all-nine-case 5% tail gate still fails: VSH remove-100 p95 rose 6.23%. This
unfavorable result is not discarded or replaced by another unchanged-code retry.

Inspection of the remaining diff pass shows that `resolve_ref` walks every ancestor
before discovering that the exact candidate is a tombstone or absent. Resolve the
exact node first, return immediately if absent, and then perform unchanged ancestor
visibility validation before exposing any present node. This uses the same lookups
for present paths and removes unnecessary work for negative lookups; it does not
cache visibility, authorize paths, load content or touch the ledger. A missing exact
node cannot be made present by any ancestor. Compare against an ancestor-first
oracle over base, overlay, hidden and nonexistent paths. This is a second production
candidate, not a tail-gate retry. Preserve the first binaries; repeat the full native
and Python ABBA contracts under `final-*` labels and report both iterations.

First-candidate native wall times (pooled p50 / p95, milliseconds):

| Workload | Before | Candidate 1 |
| --- | ---: | ---: |
| No-op | 0.137625 / 0.153958 | 0.137375 / 0.149667 |
| Read 10 | 0.803000 / 0.927708 | 0.815000 / 0.879500 |
| Edit 20 | 1.727292 / 1.853375 | 1.705166 / 1.892125 |
| Search 10K | 42.661584 / 52.496583 | 43.089375 / 45.548709 |
| Glob 10K | 50.381959 / 64.634584 | 49.980167 / 53.894750 |
| Rename 100 | 53.704584 / 58.927750 | 54.946834 / 59.992667 |
| Delete 100 | 43.036500 / 45.934250 | 44.001334 / 48.016167 |
| VSH remove 100 | 42.033042 / 46.008042 | 43.011000 / 48.875209 |
| Delete 5K | 128.005750 / 166.564875 | 127.575416 / 140.969917 |

Parent peak RSS before was 32,669,696 / 36,847,616 bytes; after was
33,341,440 / 31,899,648 bytes. The overlapping ranges do not establish a uniform
memory improvement. Required worker memory is excluded from these parent figures.

First-candidate Python/Bash wall times (100 warm samples per side, 40 cold):

| Workload | Before | Candidate 1 |
| --- | ---: | ---: |
| No-op | 0.173187 / 0.220833 | 0.170834 / 0.252000 |
| Read 10 | 0.952417 / 1.078584 | 0.990313 / 5.052500 |
| Pipeline 1 MiB | 8.081188 / 8.413084 | 8.123980 / 13.389833 |
| Edit 20 | 1.856938 / 2.438375 | 1.853688 / 3.483333 |
| Copy 1 MiB | 3.643084 / 4.073208 | 3.675521 / 6.877750 |
| Chmod 20 | 13.009813 / 15.445583 | 13.496792 / 17.586333 |
| Cold | 5.719125 / 9.270583 | 5.594521 / 11.789459 |

All Bash state/path/call/byte counters agree. Tails regressed in this series, and the
one-write state frame did **not** demonstrate a durable-I/O speedup: chmod bind/store
median was 11.667604 → 12.014480 ms. This negative result remains evidence; fewer
syscalls is not a claim of lower end-to-end latency. The next measured candidate
changes VFS lookup, not storage durability. No machine-independent latency or memory
claim is supported by these shared-host measurements.

## Final candidate results

The exact-negative lookup change passed fresh read-only judgment: it only borrows
an exact node before checking ancestors; it does not load content, expose a hidden
node or record effects. The independent ancestor-first oracle and all regression
tests pass. Final native SHA-256 is
`ee8f099fc5c35fbd027cb0181e1b4cc68e2adee17800ef5e29cbe8199a638651`;
final Python extension SHA-256 is
`db1b9d25dc1318ddfe2b27db29249b8f4d228d88bade18cbae66cde110062196`.
Both are retained as `native-final` and `python-native-final.so` in the new evidence
directory. The installed extension matches that final hash after the ABBA restore.

The complete final series is `final-native-{before-A,after-B,after-C,before-D}.json`.
All nine cases retain 200 observations per side; states and changed-path counts
agree throughout. The same warm-up, worker, harness, permission context and quantile
definition apply. No builds, tests or samplers overlapped the timed processes.

| Workload | Before p50 / p95, ms | Final p50 / p95, ms | p50 change | p95 change |
| --- | ---: | ---: | ---: | ---: |
| No-op | 0.138500 / 0.157584 | 0.138667 / 0.154583 | +0.12% | -1.90% |
| Read 10 | 0.818375 / 1.047875 | 0.792625 / 0.959125 | -3.15% | -8.47% |
| Edit 20 | 1.746500 / 2.149459 | 1.665541 / 1.769541 | -4.64% | -17.68% |
| Search 10K | 43.862500 / 49.428792 | 43.245875 / 51.041583 | -1.41% | +3.26% |
| Glob 10K | 50.526542 / 55.150500 | 50.290250 / 58.673416 | -0.47% | +6.39% |
| Rename 100 | 53.922209 / 58.338041 | 53.966750 / 63.353750 | +0.08% | +8.60% |
| Delete 100 | 43.071167 / 47.996500 | 43.011375 / 46.931792 | -0.14% | -2.22% |
| VSH remove 100 | 42.049666 / 46.201209 | 43.093417 / 52.905750 | +2.48% | +14.51% |
| Delete 5K | 127.999583 / 145.403958 | 129.942375 / 150.506750 | +1.52% | +3.51% |

Delete-5K diff per-run medians are 5.290208/5.220084 ms before versus
3.333291/3.281416 ms after (~37% less stage time). Canonical-diff allocation/traversal
work is reduced, but the total request is **not** 37% faster. The second lookup change
does not have an isolated measured speedup claim. Rename diff remains dominated by
lazy content capture/durable blobs (~11 ms); bind/store is ~12–13 ms. Required
snapshot and storage work still dominate these workloads.

The final controlled **5% all-nine-case p95 gate fails** on glob, rename and VSH
remove-100. Earlier historical pre-Bash gate failures are not superseded. No
unchanged-code reruns, selected samples or relaxed gates were used to obtain a pass.

Parent peak RSS was 33,783,808/32,653,312 bytes before and 31,948,800/32,096,256 after.
This series is lower, but the first series overlapped. No uniform RSS or process-tree
memory improvement is claimed. The worker is unchanged and excluded from parent RSS.

Final public Python/Bash results (`final-bash-*`, 100 warm and 40 cold samples/side):

| Workload | Before p50 / p95, ms | Final p50 / p95, ms |
| --- | ---: | ---: |
| No-op | 0.185584 / 0.238792 | 0.186792 / 0.236583 |
| Read 10 | 0.960938 / 1.171042 | 0.987646 / 1.137083 |
| Pipeline 1 MiB | 7.987042 / 8.418334 | 7.971521 / 8.372000 |
| Edit 20 | 1.802375 / 2.087583 | 1.815146 / 1.978084 |
| Copy 1 MiB | 3.511375 / 3.663583 | 3.511021 / 3.683083 |
| Chmod 20 | 13.003417 / 14.109917 | 12.946105 / 15.366084 |
| Cold | 5.823937 / 6.062584 | 5.881646 / 9.879542 |

All state/path/call/byte accounting agrees. Chmod stage medians: snapshot
0.162188→0.157375 ms, execution 0.901334→0.870313, diff 0.017209→0.015938, policy
0.007333→0.006980 and bind/store 11.883625→11.841958. Durable-I/O latency is effectively
unchanged; the p95 worsened. There is no general Bash latency-win claim.

## Final verification and handoff

Final workspace coverage: **293 Rust tests**, 18,181/21,519 lines (84.49%),
1,524/1,972 functions (77.28%), 28,807/33,932 regions (84.90%). Existing floors and
exclusions are unchanged. The final rebuilt Python binding passed **182 tests** with
664/664 statements and 178/178 branches covered (100% each). Final release workspace
Clippy (`-D warnings`) and Rust formatting pass; Python lint/type checks are unchanged
from the passing first candidate. Security and coverage work is complete; the
performance acceptance gate remains open as described above.

The next performance decision needs isolated-runner evidence separating fresh
snapshot metadata scans, per-blob durable capture, and artifact/log synchronization.
These are actual safety/durability boundaries, not checks to remove. Batching durable
blob installation would need its own crash-consistency design and fault-injection
tests; it is not silently introduced here. No dependency or storage format changed.
No commit, push, hosted CI, package publication or release was performed.

Documentation acceptance: regenerated `llms.txt`, `llms-full.txt` and copy-as-Markdown
assets; strict Zensical build passes. The documentation checker reports 38 pages,
51 Python snippets and 3,986 local links with zero errors. Execution feature-boundary
checks pass. After the benchmark swapped extensions, all seven Python policy
regressions passed again against the restored final binding. `git diff --check`
passes. Existing unrelated worktree changes were preserved.
