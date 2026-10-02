# Bashkit integration — evidence, artifacts and active parent bounds

Updated: 2026-10-01. Local macOS/arm64 preparation against VSH `8550facbe94485cb12bfe007fbb3575a168b6afc`.

This is a **historical preparation checkpoint**, not the current API status or release authorization. At this checkpoint the public runtime executed Monty only. Public Bash dispatch, binary artifacts and Python selection were implemented subsequently; see the [current integration evidence](bashkit_p7_p9_evidence.md). Measurements below retain their original scope rather than being relabeled as final results.

## Execution evidence and transaction identity

- `ExecutionEvidenceDigest` is a typed 32-byte digest. A binding without it retains the original transaction-v1 identity; a binding with it uses transaction-v2. Existing V1/V2 pending data is not silently re-signed.
- Current Monty execution binds complete review evidence, raw intent, ordered effects and their origins, dependency and diff context, risk/denial evidence, the tagged Monty value, stdout and execution counters. Completeness/truncation claims are themselves bound.
- The eventual hook response, transaction ID, lifecycle state, wall-clock timings and the `full_detail` display choice are excluded. Changing presentation does not redefine approval identity; changing the substantive review evidence does.
- Hashing uses a bounded 4 KiB streaming buffer. Monty values are encoded through a Postcard flavor without a body-sized temporary `Vec`; artifact serialization tees the actual encoded bytes into the seal instead of encoding/hashing the same receipt payload a second time.
- Review capture performs a borrowed, bounded size preflight before cloning retained evidence. It does not construct a large provisional serialization merely to check its size.
- Ephemeral AutoApproved previews use a counting encoder through the exact same artifact field writer as persistence. Their precise encoded-byte quota and evidence-seal checks are unchanged, but no temporary full artifact buffer is retained. Durable persistence still emits bytes. Tests prove size/error parity, zero counting-buffer capacity, exact aggregate bounds, result limits and tampered-seal rejection.

## Pending artifact V3

`VSHPND03` carries the execution-evidence seal and a versioned, tagged Monty output. Loading recomputes the seal, transaction binding, canonical diff and review projection before admitting the pending receipt. Unknown tags, malformed lengths, over-limit values and altered substantive evidence fail closed. A display-only timing change is intentionally not an evidence change. The length-delimited Postcard result must be consumed completely: V1, V2 and V3 reject an inner trailing byte even when the decoded value and its recomputed seal would otherwise be unchanged.

V1/V2 readers retain their original identity domain and mark incomplete historical review evidence honestly. They do not infer a backend from the program text. New metadata effects, Bash effect origins and permission risks are admitted only by V3, not by a relaxed historical decoder. Only the Monty output tag is currently implemented; a byte-authoritative Bash stdout/stderr envelope remains future work.

Virtual after-state modes may differ from a retained host stamp for a supported mode-only change. Before-state and read/write precondition validation remain strict. This distinction allows a durable preview of a virtual chmod without fabricating a new host stamp.

## Active parent evidence is bounded during execution

Artifact limits alone were insufficient: retained paths and observations could already grow before final capture. The new limits apply inside the parent-owned `VirtualFs` and shared adapters:

| Limit | Default | Charged retention |
| --- | --- | --- |
| `max_evidence_records` | 250,000 | Each retained effect, read dependency, write precondition or policy denial |
| `max_evidence_bytes` | 64 MiB | Retained path/origin/rule bytes and conservative record/tree/vector storage; write preconditions also reserve their overlay slots |

VFS evidence reservations precede owning path copies and vector growth. Adapter denials are reserved before retention/vector growth; the policy has already constructed one temporary structured denial. Repeated dependency keys are not cloned merely to find an existing entry. A limit/allocation failure is sticky: catching a guest exception, performing a no-op rename, raising a later request limit or running a new no-filesystem program on the failed VFS cannot turn the partial virtual state into a committable proposal. Fresh transactions remain usable. `VirtualFs::exists` therefore returns `Result<bool, VfsError>`; a resource failure is not reported as ordinary nonexistence. The Python guest `Path.exists()` surface is unchanged.

All actual parent-retained policy denials use the same cap, including denials the guest catches. Traversal-filter denials that are not retained do not invent records. Existing OS-call, read-byte, write-byte and directory-entry counters keep their original meanings.

Recursive copy/mkdir/remove/rename and walk/list/subtree frontiers have scoped byte preflights. Temporary frontier entries do not pretend to be retained evidence records. Direct-child and subtree merges borrow ordered base/overlay keys, check the byte budget, and only then clone accepted paths. Rebased rename/chmod backing origins are charged using their original lengths, not a potentially shorter destination.

The accounted byte cap is not an exact process-RSS cap. Snapshot manifests, blob storage, intent/output payloads and other parent allocations have separate bounds. Nested temporary scopes are independently checked against available evidence bytes; this does not claim aggregate whole-process memory confinement. Synchronous parent traversal/lazy loaders also remain non-preemptible by the worker watchdog.

The shared gateway descriptor is now `vsh-fs-gateway-v3`; the Monty configuration domain remains `vsh-monty-config-v5` and includes the gateway descriptor. Both new limits participate in Monty and private Bash configuration identity. Gateway v3 additionally binds the stable directory-observation semantics described below.

## Directory hashing without a full canonical payload allocation

Directory-v1 digests keep their exact original domain, payload-length prefix and entry codec. A replayable ordered iterator supplies the length pass followed by a streaming hash pass; the only per-node encoding scratch is bounded metadata, not directory contents. The base snapshot no longer clones child paths just to build its digest.

Compatibility tests compare the streaming digest against the original canonical byte construction for empty directories, files/directories/symlinks, blob/stamp content, optional ctime, Unicode paths and lengths around the 4 KiB buffer boundary. The byte encoding is unchanged. Review identified a separate concurrency flaw: shared lazy capture could change a node from Stamp to Blob between the two streaming passes. Directory observations now use immutable virtual node state, not blob-cache state. Explicit overlay mode/content/path changes remain visible; loading unchanged bytes is not a directory mutation. A concurrent cloned-snapshot regression test covers capture stability and chmod/write/rename, while existing golden tests retain codec parity.

With an empty overlay, the visible and base-expected directory digest are therefore identical and computed only once. An already-recorded base-directory observation is reused across later listings; each visible listing still produces its own truthful effect digest. No snapshot-sized state array or additional synchronization was introduced.

## Adversarial coverage added

- Long repeated missing paths and exact byte/record boundaries; overflow, tightened limits, no refund and sticky terminal failures.
- Directory/subtree temporary limits without charging fake evidence records; long original backing origins after a short rename.
- Long recursive-copy destinations and recursive-mkdir frontiers fail during preflight, before overlay mutation or ancestor-buffer growth.
- Denial record and byte caps cover retained path/rule data without producing filesystem effects or fabricated I/O counters.
- Real Monty/Bash workers perform a partial virtual write and catch repeated missing/protected reads. Parent failure remains terminal; no canonical diff or reuse of that VFS is admitted. A fresh worker transaction succeeds.
- Python Auto-mode resource failure leaves the host unchanged and does not prevent a later fresh preview.
- Raw artifact tampering, restart preservation, legacy identity, mode after-state validation, bounded decoding, streaming value-codec parity and timing/display exclusions.

## Prior-checkpoint local checks

The table below records the checkpoint before the subsequent review fixes. Current verification is recorded separately below; the historical coverage percentages are not a fresh measurement of the added cancellation/profile branches.

| Check | Result |
| --- | --- |
| Rust workspace, all features/targets, locked/offline, LLVM-instrumented | 259 passed; none ignored |
| Rust LLVM line/function/region coverage | 83.11% / 76.03% / 83.72%; existing 79 / 70 / 81 floors passed |
| Rust formatting and Clippy `-D warnings` | Passed |
| Rustdoc, all features, no deps, existing runtime exclusion | Passed with `-D warnings` |
| Rebuilt release PyO3 extension, CPython 3.14.6 | Passed |
| Six current Python runtime/API/hook/capability suites | 155 passed |
| Python `src/vsh` line and branch coverage | 100%; 566 statements, 152 branches |
| Ruff and both ty/basedpyright | Passed |
| Documentation validation | 37 pages, 3,772 local links, 48 Python snippets; no errors |
| Zensical build and generated `llms`/Markdown consistency | Passed |
| Release version consistency | 0.5.0; no version bump |
| Default Rust consumer graph | No Bashkit or Tokio |
| Worker-only graph | No VSH execution/VFS/policy/runtime/commit authority |
| Locked dependency audit / deny | No vulnerability advisory; deny passed; maintenance warning below |

The first Python run after generated-cache cleanup failed to discover the removed debug worker. The complete rerun explicitly selected the rebuilt release worker with `VSH_MONTY_WORKER`; all 155 tests passed without relaxing timeout, coverage or assertion settings. No hosted Linux/Windows result is claimed.

Audit used the existing local RustSec database with 1,277 advisories and checked 285 locked dependencies. It still reports `atomic-polyfill@1.0.3` / `RUSTSEC-2023-0089` as an existing all-target maintenance warning. No new advisory ignore or license/duplicate exception was added by this checkpoint. Passing audit is not a permanent CVE guarantee.

Rust LLVM coverage includes 16,282 / 19,590 lines, 1,405 / 1,848 functions and 25,791 / 30,805 regions. The existing `(vsh-python|vsh-worker)` filename exclusion was retained; no Bash file was excluded and no floor was lowered. The coverage-only Bash child environment forwards the LLVM profile path so its actual subprocess execution is counted; production guest environment rules are unchanged. These line/function/region percentages are not Rust branch coverage. CI now supplies the `--json` format required by cargo-llvm-cov 0.9.0's `--summary-only` option and writes its summary to `target/rust-coverage.json`.

## Native performance measurements

### Prior checkpoint (before single-pass fresh sealing)

Final release binaries ran serially in A/B/B/A order, without concurrent VSH compilation or tests. Each run used 100 warm iterations per case, five cold iterations and two parallel workers. The table pools 200 warm samples per case/version; percentiles use the existing nearest-rank index `floor((n - 1) * q)`. The same rebuilt release Monty worker served both binaries.

Raw JSON and matching Markdown reports are under `target/bashkit-evidence/p6/`: `final-baseline-a`, `final-candidate-b`, `final-candidate-c`, `final-baseline-d`. The earlier `bounds-baseline-a` / `bounds-candidate-b` pair is exploratory data from before strict result decoding and the counting-preview optimization, not the final candidate. Historical pre-buffer/tee reports are retained separately.

| Case | Baseline p50/p95 (ms) | Current p50/p95 (ms) | p50 / p95 change |
| --- | --- | --- | --- |
| noop | 0.196 / 0.304 | 0.260 / 0.446 | +32.5% / +46.4% |
| read_10 | 1.046 / 1.862 | 1.010 / 1.329 | -3.4% / -28.6% |
| edit_20 | 2.169 / 4.124 | 2.080 / 2.581 | -4.1% / -37.4% |
| search_10k | 79.211 / 114.788 | 65.662 / 71.656 | -17.1% / -37.6% |
| vsh_glob_10k | 79.154 / 85.527 | 75.582 / 80.351 | -4.5% / -6.1% |
| rename_subtree_100 | 79.176 / 108.732 | 71.903 / 96.167 | -9.2% / -11.6% |
| delete_subtree_100 | 67.058 / 80.486 | 60.699 / 158.069 | -9.5% / +96.4% |
| vsh_remove_subtree_100 | 65.354 / 84.084 | 71.578 / 131.954 | +9.5% / +56.9% |
| massive_delete_5k | 159.080 / 288.721 | 182.929 / 344.986 | +15.0% / +19.5% |

**The original performance acceptance gate is not closed.** These wall-time results contain substantial host drift and are not validated speedup claims. Large-delete baseline p50 moved from 145.367 ms in A to 197.817 ms in D; candidate p50 moved from 165.866 ms in B to 240.265 ms in C. Candidate no-op p50 also changed from 0.340 ms to 0.193 ms. Background WindowServer/iTerm CPU activity was observed; that observation and the control drift undermine attribution, not the recorded failures. In particular, the no-op and several deletion p95 comparisons do not meet the planned regression bound.

There is also a repeatable binding-stage cost to investigate, not just noise: glob binding p50 is about 8 ms in both candidate runs versus about 3.5 ms in baseline A. Counting previews demonstrably remove the full temporary encoded buffer, but these measurements do not isolate a latency gain from that optimization or prove reduced total RSS. The new evidence seal and preflight remain mandatory; they must not be bypassed to satisfy a benchmark.

Remaining optimization work: profile the isolated binding stage, quantify repeated immutable evidence/value traversal and dependency-map lookup costs, and measure attributable peak allocations. Only then choose a cache/codec/index optimization with exact digest/error/quota parity. Repeat short interleaved control measurements on a stable host before closing P2; do not infer a speedup from the favorable pooled rows.

Reproducible binary SHA-256 identifiers:

| Binary | SHA-256 |
| --- | --- |
| Cached baseline | `1e7ac584a8f59eeed36fa902b2780d41be1e5d60b08b01d0403e48ecde4103e8` |
| Current native benchmark | `c46d1302bd573ea0fedee748eace8c88d0ad4c76b03c4562e4497541cd1998a1` |
| Shared release Monty worker | `c571b3c5844b979763a7bcb410f1ddf8e4184ebab59969e2819fd74371377737` |

## Review and remaining gates

Independent read-only active-evidence review found six concrete issues; all were source-fixed and rechecked: backing-origin length accounting, borrowed resolution before temporary cloning, true temporary-byte rather than pseudo-record charging, bounded parent frontiers, denial retention and upfront sticky-VFS checks. A separate seal/artifact review identified and then rechecked the inner Postcard trailer fix; it also checked the counting encoder without finding a concrete validation/accounting regression. The reproducible tests remain the behavioral authority; source review does not prove a global RSS bound.

Public Bash is still gated by byte-authoritative Bash evidence/results, public dispatch, Python async cancellation integration, platform mode tests and the original performance acceptance criteria. The private worker's cancellation and virtual `coproc` behavior are now implemented and explicitly profiled below. No public Bash example or capability/tool choice is advertised yet. Nothing was committed, pushed or released.

## Review fixes — 2026-10-01

- Fresh AutoApproved previews now seal evidence and count the entire V3 artifact in one pass. Creation rejects already-sealed input, updates identity only after all bounds pass, and never refreshes persisted evidence. Promotion, ordinary artifact encoding and decoding still verify the seal. Exact bytes/digest/transaction/size parity, exact limits, failed-creation identity preservation and later tamper rejection are tested. Other lifecycle branches retain their existing sealing path.
- Repeated metadata observations no longer clone an `Arc` and re-read immutable base state. Repeated sibling writes reuse a borrowed parent lookup before allocating a new parent path. Charges and sticky failures are unchanged. Monty unlink/rmdir authorization runs at the shared gateway instead of redundantly before and inside it.
- `BashCancellation` is a request-scoped private-backend handle. The host checks it at entry, worker checkout, bounded pipe waits, before/after filesystem dispatch, reset reception and final success. Cancellation retires the lease and cannot produce a successful result. Tests cover pre-cancellation, running cancellation, queued checkout, late channel success and clean subsequent execution. This is cooperative cancellation: arbitrary synchronous host loaders are checked on return, not forcibly interrupted. Python public async wiring remains P7 work.
- Private worker/profile identity advanced to revision 2. Unsupported network commands and successful no-op builtins fail terminally, including caught/nested/wrapped calls. `env` and `tar` are excluded wholesale; `cp`/`mv`/`rm` use narrow argv contracts, and unsupported `chmod` flags/special bits are rejected before they can be silently ignored. The system design lists the exact restrictions.
- Synchronous virtual `coproc` is supported explicitly, including nested forms, without native processes or concurrent VFS mutation. Actual process-substitution `/dev/fd` access remains terminally unsupported. This is not syntax-level rejection of unused substitution output, nor a claim that the upstream FD limit counts coprocess buffers.

Fresh source reviews found no remaining concrete blocker in these narrow seal/profile/cancellation changes. The subsequent optimization review found the shared-lazy directory race; its fix and regression test are described above. These reviews do not certify public integration or cross-platform execution. Current full locked/offline Rust workspace/all-features/all-targets tests: **267 passed, none ignored**; current Clippy `-D warnings` and formatting passed. Test builds twice exhausted disk space; only reproducible `target/llvm-cov-target` and `target/debug` build caches were removed, preserving source and benchmark/coverage reports. A clean full run then passed. Regeneration of the caches requires a build; no source recovery is needed.

### Single-pass sealing A/B/B/A measurements

Raw files: `target/bashkit-evidence/p6/review-{baseline-a,candidate-b,candidate-c,baseline-d}.{json,md}`. Same baseline and worker as above; 100 warm samples per case/run, five cold iterations, two parallel workers; serialized runs without concurrent VSH builds/tests. This series precedes the final repeated-metadata/delete-authorization micro-optimizations.

| Case | Baseline p50/p95 (ms) | Candidate p50/p95 (ms) | p50 / p95 change |
| --- | --- | --- | --- |
| noop | 0.181 / 0.222 | 0.184 / 0.221 | +1.6% / -0.1% |
| read_10 | 0.938 / 1.118 | 0.973 / 1.299 | +3.8% / +16.1% |
| edit_20 | 1.914 / 2.255 | 2.012 / 2.839 | +5.1% / +25.9% |
| search_10k | 62.798 / 65.035 | 64.917 / 70.515 | +3.4% / +8.4% |
| vsh_glob_10k | 67.391 / 69.952 | 72.190 / 77.350 | +7.1% / +10.6% |
| rename_subtree_100 | 70.625 / 80.173 | 70.451 / 78.768 | -0.2% / -1.8% |
| delete_subtree_100 | 58.945 / 63.312 | 59.972 / 65.240 | +1.7% / +3.0% |
| vsh_remove_subtree_100 | 58.006 / 60.354 | 59.006 / 64.772 | +1.7% / +7.3% |
| massive_delete_5k | 158.531 / 202.487 | 187.393 / 243.523 | +18.2% / +20.3% |

Glob binding medians fell from the prior candidate's approximately 8.0/8.1 ms to 5.48/5.28 ms, consistent with removing one value/evidence traversal. These are separate runs, not an isolated causal speedup or a total-RSS measurement. The paired baseline remains about 3.24/3.25 ms. The **5% end-to-end regression gate remains unmet**; correctness fixes are not blanket merge/release approval.

The release PyO3 extension was rebuilt again after the directory fix. All **155 Python tests passed in 21.42 seconds**, retaining **100% line and branch coverage** (566 statements, 152 branches). No live judge/model requests were used.

### Final directory/dependency fixes — candidate E, control F

Raw files: `review-final-candidate-e` and `review-final-baseline-f` in the same evidence directory. Each has 100 warm samples per case, five cold iterations and two parallel workers. Runs were serialized after builds/tests completed. E includes the immutable-directory fix, shared empty-overlay digest, metadata lookup changes and removal of duplicate delete authorization. F is the unchanged cached baseline. This follow-up pair is not pooled with earlier candidates with different code.

| Case | Control F p50/p95 (ms) | Final E p50/p95 (ms) |
| --- | --- | --- |
| noop | 0.193 / 0.235 | 0.335 / 0.434 |
| read_10 | 0.997 / 1.476 | 0.946 / 1.850 |
| edit_20 | 1.881 / 3.490 | 1.924 / 3.501 |
| search_10k | 62.008 / 63.758 | 60.483 / 62.083 |
| vsh_glob_10k | 66.559 / 69.131 | 69.732 / 73.255 |
| rename_subtree_100 | 69.353 / 71.490 | 70.488 / 78.956 |
| delete_subtree_100 | 58.942 / 63.990 | 60.071 / 73.919 |
| vsh_remove_subtree_100 | 57.889 / 59.043 | 63.049 / 100.732 |
| massive_delete_5k | 141.305 / 152.894 | 168.880 / 202.217 |

Glob execution p50 is now 18.61 ms (previous candidate B: 21.26 ms; final control F: 18.18 ms), while its binding stage remains 5.15 ms versus 3.10 ms. Large-delete execution and binding medians remain above control: 77.99/29.41 ms versus 68.06/19.95 ms. Control drift is material: the same baseline's large-delete p50 moved from 167.20 ms in D to 141.31 ms in F. Therefore the apparent reduction from an earlier candidate's 187 ms to final E's 169 ms does not establish end-to-end non-regression. No-op also varies significantly across runs despite unchanged no-op logic.

Final candidate SHA-256: `afd09a34048fd7495630fcedf961751c689a0728ada3332a0548603498ad990c`. Baseline and shared worker hashes are unchanged from the table above.

### Approval decision

The implemented correctness/profile/cancellation fixes pass local tests and independent source review. **Full merge/release approval is withheld:** the original 5% performance target is not met by the available measurements, public Bash result/artifact/dispatch and async Python cancellation are unfinished, and hosted platform validation has not run. The remaining binding and deletion costs need focused profiling with short interleaved controls; do not waive evidence checks, relax limits, lower coverage floors or relabel the current private backend as a completed public integration to obtain approval. No commit, push or release was performed.
