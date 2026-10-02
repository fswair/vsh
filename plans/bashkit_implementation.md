# Bash execution on VSH — implementation plan

Status: public Rust/Python integration, byte-authoritative Bash artifacts, hooks, capability/MCP/CLI dispatch and cooperative cancellation are implemented. Profile revision 4 fails closed on unsupported or lossy byte conversions. Final performance, artifact and platform verification is recorded in the [completion evidence](bashkit_p7_p9_evidence.md); historical private-worker checkpoints are not the current API status. The existing published version remains 0.5.0; no publication is authorized by this plan.
Updated: 2026-10-02.
Design authority: [system design](bashkit_system_design.md).
Baseline: VSH `8550facbe94485cb12bfe007fbb3575a168b6afc`.

The system design explains ownership and semantics. This file defines work order, touched areas, methods, acceptance tests and stop conditions. Proposed commands/types are explicitly identified; they are not promises that new packages already exist. No phase grants permission to publish, push, switch branches, overwrite unrelated work, or change the JEV policy.

## 1. Execution rules and phase map

Follow the repository's `plans/` convention; keep these two documents as durable plans. Keep generated logs, fixture trees, benchmark traces and review scratch under ignored `target/bashkit-evidence/` or the existing local benchmark output directory. Do not add another playground or commit bulk generated results as source.

At each phase: inspect the worktree; preserve unrelated changes; implement only that phase; run narrow checks before full checks; review against the system invariants; record actual evidence and unresolved findings. Update this plan before deviating from the contract. Never mark a later phase complete because an earlier smoke test passed.

### Active parent-evidence resource checkpoint

The artifact-size check at completion is not a sufficient parent allocation bound. Before public dispatch, implement an active VFS evidence budget shared by native callers, Monty and the private Bash gateway:

- Add request limits `max_evidence_records` (default 250,000) and `max_evidence_bytes` (default 64 MiB). A record is one retained effect, read dependency, write precondition or adapter-retained policy denial; write preconditions also reserve their corresponding overlay entries. These are not OS-call or file-byte counters. Filtered/ignored traversal denials are not retained and must not consume this record count.
- Charge owned path bytes and conservative container/storage costs before retaining new records. Preserve every accepted ordered effect; do not coalesce, truncate or silently omit evidence. Avoid cloning an existing read/write key on every repeated observation. Grow effect storage geometrically, not one realloc per event.
- Bound temporary directory/subtree path collection, rebased copy destinations and traversal frontiers before growth too, using byte-only reservations per live compound-operation scope. Account actual retained original paths after rename/chmod, even when destination paths are shorter. Stream canonical directory hashing with the exact existing domain/encoding and bounded scratch; its length prefix requires replayable immutable entries. The byte budget is an explicitly accounted filesystem evidence/storage envelope, not parent RSS, snapshot size, blob content, allocator metadata or a whole-process memory guarantee; nested buffer scopes are checked independently.
- A limit/allocation failure is sticky on the VFS. Reject subsequent filesystem operations and canonical diff generation; neither a caught guest error nor reuse with a fresh gateway/budget may make the partial transaction actionable. Existence checks must return an explicit error, not false on resource failure.
- Bind both limits and the changed gateway semantics into execution configuration identity. Python budget properties, stubs, MCP schemas and docs must agree with Rust. Do not alter the worker's heap limit or falsify existing receipt I/O statistics.
- Test repeated long missing paths, unique read/write paths, one-call recursive growth, zero limits, exact boundaries, overflow, normal effect order, caught failures, worker retirement and no host mutation. Rerun performance and independent source review after implementation.

This checkpoint implements the bounded-parent requirement now shared by public Monty and Bash requests. Profile review resolved `coproc` as synchronous virtual execution and tightened unsupported-command handling; see the current system design.

Measured follow-up: ephemeral AutoApproved previews use a counting encoder instead of allocating an artifact for its length; fresh sealing/counting shares one pass. Durable encoding/decoding continue verifying sealed evidence. Directory observations use immutable virtual state, gateway descriptor v3 binds that semantic, and empty-overlay visible/base digests share computation. Completed execution consumes the VFS to move read/write/effect evidence rather than clone it. Public Bash uses request-scoped cancellation and profile v4. See [historical review fixes](bashkit_p6_evidence.md#review-fixes--2026-10-01) and [current verification](bashkit_p7_p9_evidence.md).

| Phase | Deliverable | Prerequisite | Main gate |
| --- | --- | --- | --- |
| P0 | Reproducible feasibility evidence and baseline | None | Profile/limits can be enforced without a fork or host fallback |
| P1 | Frozen semantic/API/wire contracts | P0 | Explicit failure, identity, compatibility and platform decisions |
| P2 | Shared policy-aware gateway, Monty migrated | P1 | Monty semantic and performance parity |
| P3 | Mode mutation and complete evidence handling | P2 | Real atomic-edit semantics, policy and stale checks |
| P4 | Bounded Bash worker transport/lifecycle | P1–P3 | Host-owned FS, fail-closed protocol and resource handling |
| P5 | Bash execution integrated with runtime | P4 | Useful shell workflows, no partial-failure commit |
| P6 | Durable result/evidence/identity and hook parity | P5 | Restart/review/revalidation cannot cross backend boundaries |
| P7 | PyO3, Python, capability, MCP and CLI | P6 | Typed public flows, async cancellation and packaging discovery |
| P8 | Security matrix and measured optimization | P7; tests added throughout | No unresolved invariant or performance blocker |
| P9 | Documentation, artifacts and release-readiness | P8 | Installable, documented, tested product; no automatic publish |

Implementation can split a phase into small commits, but externally callable Bash remains disabled until P5/P6 gates pass. A feature branch is recommended for broad implementation; ask before creating/switching it. No separate agents or paid model calls are necessary for this planning artifact. A later independent read-only review is appropriate at the gateway, protocol and final security boundaries.

Ordering clarification: shared FS contracts, Monty extraction and private-worker preparation preceded public dispatch. Nested dispatch is guarded at resolved argv rather than through a second parser; the profile explicitly permits synchronous virtual coprocesses and rejects actual unsupported namespace access. Public runtime Bash is now host-opt-in; the default production Rust `vsh` dependency graph still contains no Bashkit/Tokio. Verification records distinguish public flow tests from earlier private-worker probes.

## 2. P0 — feasibility spike and performance baseline

### Objective

Resolve the risky assumptions cheaply before extracting production code. Establish evidence that another developer can rerun; do not rely on the previous `/tmp` research checkout.

### Work

1. Record current source revision, worktree state, Rust/Python/tool versions, target triple, OS/CPU and dependency lock digests. Record free disk before heavy builds; use a scoped target directory only where isolation is necessary.
2. Reconfirm Bashkit release/pin/advisories at implementation time. The reviewed candidate is `=0.18.2`, default features disabled. Resolve in isolation first, then test the combined VSH graph. Compare features/licenses and transitive version changes; a clean standalone audit is not a combined-workspace audit.
3. Reproduce the six observed scenarios: pipeline filtering, `sed -i`, background append, `xargs -P`, unsupported `parallel`, and writes before final `false`. Add assertions, not output-only smoke logging.
4. Build a recording/rejecting **test-only** filesystem implementing the real upstream trait. Record all methods/paths/flags for redirects, mkdir/cp/rm recursion, mode preservation, scripts, `source`, `eval`, subshells and process substitution. This fixture is an adapter-contract probe, not a new production VFS.
5. Verify profile rejection through actual dispatch: command aliases/functions, `command`, nested `sh`/`bash`, workspace executables and command-not-found handling. Determine the narrow supported upstream hook/builtin replacement mechanism. Do not parse shell source with regexes.
6. Force stdout, stderr and intermediate-data limits separately. Test a producer in a pipeline whose consumer exits 0, output redirected into a file, nested command substitution and suppressed stderr. Confirm that incomplete intermediate output cannot be reported as a complete successful mutation. Final flags alone may not capture an earlier pipeline stage's truncation.
7. Prototype only the worker memory-limit boundary using the existing pinned allocator in a throwaway worker binary. Prove hard-limit exit, baseline/headroom semantics, multi-thread allocation accounting if any, reset, OOM handling and parent survival. Measure one-task-at-a-time Tokio startup overhead.
8. Verify the minimum filesystem profile: `/workspace`, synthetic root, `/dev/null`, explicit rejection of `/tmp` and `/dev/fd`, opaque symlink reads and mode semantics. Trace denied accesses, including calls whose errors Bashkit catches internally.
9. Establish current Monty baseline using the repository's existing Rust/PyO3 benchmarks. Include cold/warm worker, reads, modifications, no-op and independent-runtime concurrency. Use isolated fixture workspaces; never benchmark destructive commands against the project or a real home directory.

### Files / outputs

- Read: upstream tagged source, `Cargo.toml`, `Cargo.lock`, `deny.toml`, existing worker/client, build wrapper, benchmark scripts.
- Temporary code/evidence: `target/bashkit-evidence/p0/`; extract reusable contract cases into owned tests only when the implementation phase begins.
- Update the decision ledger in the system plan with actual findings and exact dependency candidates; do not introduce dependency placeholders into production manifests.
- Durable upstream contract probe: `scripts/bashkit_contract_probe/`. Findings and baseline: [P0 evidence](bashkit_p0_evidence.md). This isolated, unpublished package deliberately does not add Bashkit to Monty-only builds.

### Acceptance

- All observed semantics are asserted, including the negative cases.
- A supported way exists to signal unsupported operations and intermediate-output incompleteness reliably. Otherwise stop for an upstream fix or explicitly reduce scope; do not silently vendor/fork.
- The effective allocation cap and timeout behavior are documented and demonstrable; no silent downgrade of `ExecutionBudget.max_memory_bytes`.
- Linux/macOS mode preservation works in the intended host commit model. Windows is explicitly gated if it cannot meet the same profile.
- Baseline data and reproduction commands exist; no optimization percentage is claimed yet.

Suggested existing commands, after following the repo's environment setup:

```sh
cargo test --workspace --all-targets --locked
cargo audit --file Cargo.lock
cargo deny --all-features --locked check
uv run python benchmarks/native_pyo3.py --iterations 200 --cold-iterations 30 --output benchmarks/results/local/bashkit-baseline.json
```

The benchmark invocation is for baseline generation, not a test of Bash support. Use the existing native benchmark's documented invocation after checking its current flags; do not invent a new benchmark framework.

## 3. P1 — freeze contracts before broad refactoring

### Objective

Eliminate semantic ambiguity before adding types throughout Rust/Python.

### Work

- Finalize `Language`, `BashConfig`, result/error shapes and optional-feature ownership using compile/type fixtures. Keep Monty as the request default and Bash host-opt-in. Preserve direct-argument/request preview overloads and capability constructor-first usage.
- Define the exact initial profile: namespace, supported mode mask, symlink behavior, timestamps, disabled commands and flags, sequential jobs, output encoding, time/randomness disclosure and nested-script budgeting.
- Define one error taxonomy: recoverable FS error; hard policy denial; profile unsupported; final shell nonzero; guest resource failure; parent resource failure; protocol/mismatch; cancellation; stale/commit failures. Specify which yields a denied complete receipt versus a noncommittable execution error.
- Write the FS operation/authorization matrix against existing `AccessKind` variants. Define whether a compound action has all-path preflight and how every failed/denied operation remains visible. Preserve documented Monty behavior.
- Allocate stable wire/artifact and identity-domain tags, session states, frame/count limits, reset rules and descriptor fields. Define the noncircular execution-evidence digest inputs. Unknown version/tag is fatal; never try another decoder until one “works.”
- Map every common request budget to effective Bash limits. Use host ceilings and explicit byte/count/time units; no request can raise a host ceiling. Separate parser/work/output/intermediate limits from filesystem growth/total work accounting.
- Set upstream `max_stdout_bytes` and `max_stderr_bytes` to `usize::MAX`: these knobs silently truncate nested producer data in 0.18.2. Enforce bounded intermediate work/allocation and parent-retained output as terminal failures instead. Never use `after_tool`/`after_exec` output hooks: their string projection corrupts non-UTF-8 bytes. A before-dispatch guard and deny-only `CommandResolver` observer retain unsupported/unresolved attempts even when the shell handles the error.
- Decide synthetic-root and `/dev/null` evidence representation. They must be distinguishable from workspace mutations and must not require fake `VPath`s that could later be committed.
  Proven correction: upstream null-device redirects bypass the FS trait. Interpreter work/allocation/deadline bounds cover them; only actual FS RPC calls enter FS accounting. The probe also proves `before_exec` does not observe nested parses. The reviewed v4 profile accepts synchronous virtual `coproc`; actual `/dev/fd` access from process substitution remains a sticky terminal namespace failure. This is not syntax-level rejection of an unused substituted path. A missing slash-path workspace script is a recoverable filesystem failure and does not invoke `CommandResolver`; do not mislabel it as an intercepted unknown external command.
- Define upgrade/rollback behavior for old pending approvals and in-progress host commits. Keep old V1/V2 data recoverable; reject unverifiable approvals rather than re-signing them.
- Record intentional Rust receipt-field changes and the exact Python compatibility behavior. Do not promise a backward-compatible Rust struct-layout change.

### Acceptance / review

A reviewer can determine the outcome of every row in Sections 7–10 of the system design without guessing. No unresolved question remains about partial writes, denial catchability, binary output, mode-only commits, request identity or cancellation. Update both plans if P0 invalidates a proposed choice.

No production behavior change is required in this phase; small contract fixtures are appropriate. Public exports are not added merely as placeholders.

## 4. P2 — shared filesystem gateway, without Monty regressions

### Touched areas

New `crates/vsh-execution/`; `crates/vsh-monty/src/lib.rs`, `tools.rs`, `worker.rs`; workspace manifests; focused tests in those crates. `vsh-policy` remains the owner of `CallPolicy`.

### Work

1. Characterize existing Monty direct-OS/high-level-tool behavior: paths, read/metadata/directory observations, protected children, denied accesses, recursive operations, budget accounting and effect origin.
2. Move only neutral path mapping, FS accounting/stats/error data and policy-aware operations into the shared crate. Keep Monty object conversion, interpreter settings and tool-specific argument parsing in `vsh-monty`.
3. Implement a borrowed gateway with no global mutex, dynamic plugin registry or VFS cloning. Keep origin as an explicit per-dispatch scope; restore it after errors. Do not expose `&mut VirtualFs` to untrusted/adaptor extension code.
4. Route both in-process Monty and subprocess Monty through the same semantics. Ensure high-level `vsh_copy`/remove/glob/search cannot bypass child authorization or consume uncharged work.
5. Preserve missing-path reads and write preconditions. Internal paths stay protected in direct calls, listings and recursion.
   Explicit security correction: the old Monty rename authorized only subtree roots. The shared rename preflights every source and rebased destination child, charges directory traversal, and records those observations before mutation. Protected-child and rebased-path limit failures leave the subtree unchanged. This deliberately adds traversal evidence/work to directory rename; it is not an incidental compatibility drift. Path validation now precedes shared write/append charging for in-process requests with both an invalid path and an oversized payload; subprocess decode caps remain early.
   Bind `vsh-fs-gateway-v1` into Monty's configuration under `vsh-monty-config-v4`; old pending handles must not silently inherit the stronger rename semantics. The later backend/evidence identity work remains required and does not replace this boundary version.
6. Track operation count, directory entries, bytes, path length and work before expensive allocation. No zero/overflow limit wraps to “unlimited.” Avoid charging the same byte twice by accident; distinguish logical guest bytes from actual reconstruction/materialization work. Keep live-capacity gauges separate from monotonic work counters; deletion cannot refund already consumed work. Test delta-based usage against recomputed totals and define base-versus-growth limits explicitly.
7. Keep errors backend-neutral at this seam; translate to Monty exceptions only in the Monty adapter.

Implemented listing boundary: `VirtualFs::read_dir_with_limit` enumerates ordered base/overlay entries with a streaming merge instead of allocating a full candidate BTreeSet. The gateway supplies the remaining cumulative entry budget. Over-limit enumeration stops at the first excess entry, returns a hard limit error, and never exposes a successful partial listing. Rejected listings do not emit `DirectoryRead` effects; the directory metadata dependency is still retained. Hidden names count toward the cap before policy filtering. This is a deliberate bounded-work change included in the gateway version above.

### Acceptance

- Existing Monty suites pass without weakening assertions or changing accepted policy states incidentally.
- Characterization tests prove old/new equivalence for observations/effects/denials, not only final content.
- Repeated denied calls cannot exhaust unbounded evidence buffers; a bounded terminal failure stops the run.
- Rust Monty-only benchmark and dependency graph do not acquire Bash/Tokio work. Record paired measurements against P0.

Future verification commands after the new crate exists:

```sh
cargo test -p vsh-execution -p vsh-monty -p vsh-runtime --locked
cargo clippy -p vsh-execution -p vsh-monty --all-targets --locked -- -D warnings
```

Review this phase independently before attaching Bash; a correct final diff does not prove the authorization extraction is correct.

Local evidence: [P2 gateway implementation and verification](bashkit_p2_evidence.md). Production workspace: 196 Rust tests; release Python surface: 153 tests and 100% line/branch coverage after rebuilding the binding. Rust coverage floors were not changed. Default Rust graph contains no Bashkit/Tokio. Release manifests, artifact count and publish ordering include the new shared crate; this does not authorize a release. The noisy no-op/tail-latency benchmark gate is explicitly still open.

## 5. P3 — modes, atomic edits and evidence completeness

### Touched areas

`crates/vsh-vfs`, `vsh-types`, `vsh-policy`, `vsh-commit`; mode/effect encoding in `crates/vbash/src/artifact.rs` and review projection code. Coordinate the new artifact writer with P6 so partially implemented formats cannot be produced by a released API.

### Work

- Implement a single mode-change primitive with content/type preservation and metadata read/write preconditions. Add a semantic effect, e.g. `ModifyMetadata`, with stable encoding; never mislabel mode changes as content writes.
- Reject unsupported special bits and link traversal before touching state. Handle unchanged mode as a semantic no-op with required authorization/observation/accounting.
- Verify existing commit planning actually installs the intended file and directory modes. Extend it only where demonstrated necessary; preserve descriptor-relative/capability-rooted host operations.
- Decide policy behavior for canonical permission changes. Default bounded profile requires review for meaningful final permission changes, including executable-bit grants; protected changes remain denied. A transient mode-preserving temp-file step must not automatically turn every safe `sed -i` into an extra judge call merely because an internal chmod occurred.
- Implement/test error recovery of sibling-temp write → chmod → rename replacement. Failure leaves the original virtual target intact for that primitive; earlier script changes remain subject to the final execution-failure rule.
- Update every exhaustive effect match, summary, serialization and review renderer. Byte-limited evidence records incompleteness truthfully.
- Test stale mode changes and combined content+mode changes during approval. Review must see both canonical state and relevant temporary operations.

### Acceptance

`sed -i` on 0600 and executable files preserves mode after real commit in a temporary host workspace. Standalone chmod produces correct canonical metadata diff, policy and review evidence. Setuid/setgid/sticky attempts cannot be committed. Stale mode and content changes are caught. Native content is not unnecessarily read just to modify a mode where a metadata-only path is valid.

```sh
cargo test -p vsh-vfs -p vsh-policy -p vsh-commit -p vsh-runtime --locked
```

## 6. P4 — worker package, protocol and process control

### Touched areas

New `crates/vsh-bash/` with cohesive `protocol`, `client`, `filesystem`, `guest` modules and a small `src/bin/vsh-bash-worker.rs`; workspace pins/features; new protocol/process tests. Do not add a generic transport framework or change Monty's upstream protocol.

### Work

1. Add exact-reviewed pins and no-default Bashkit feature selection. Separate default host code from optional worker engine dependencies; worker-only builds must not activate parent runtime/committer code.
2. Implement framing with an early length cap and allocation-aware nested validation. Use request/session IDs, an explicit state machine and one outstanding FS call. Reject trailing data, wrong direction and unauthorized message kinds.
3. Implement RPC `FileSystem` using only typed requests. No local writable overlay, Bashkit default in-memory fallback, RealFs or direct host path use. The worker never receives a workspace directory handle or commit handle.
4. Implement parent dispatch into the gateway and bounded responses. Check lengths/types/paths independently of any upstream validation. A compromised/malformed worker must not submit arbitrary effects or a final diff.
5. Configure fresh interpreter, minimal env, virtual cwd, bounded interpreter/parser/work limits and allocator. Initialize allocator tracking before guest allocations; test baseline/reset/headroom explicitly.
6. Implement lifecycle: spawn without shell; approved binary selection; handshake; execution; graceful reset; compatible pool return. Drain stderr and protocol independently. Bound queue size and outbound/inbound buffering.
7. Implement parent wall timeout/cancellation and kill/reap. Budget work while draining a flooding child. Never pool a child after failed reset, protocol violation, panic, timeout or memory-limit termination.
   The feasibility loop exceeded a 20 ms cooperative timeout by roughly 100 ms; the parent deadline is authoritative, not the upstream tracker.
8. Verify nested commands share limits and the same FS proxy. Worker process cache reuse must not reuse variables, cwd, functions, file descriptors, captures or hooks from another transaction.

### Acceptance

- Fake-worker tests demonstrate safe rejection of oversized/truncated frames, invalid nested lengths, illegal IDs, double completion, FS call after finish, exit mid-request, stderr flooding and reset failure.
- Real worker can perform gateway reads/writes and report binary output without touching the fixture's host workspace before commit.
- Allocator-limit death and timeouts terminate only the worker and are typed execution failures; parent state is bounded and noncommittable.
- Worker A/B sequential reuse and concurrent independent-runtime tests find no state contamination.
- No process/global allocator change occurs in the Python extension or parent Rust runtime.

Future commands:

```sh
cargo build -p vsh-bash --no-default-features --features worker --bin vsh-bash-worker --locked
cargo test -p vsh-bash --features worker --locked
cargo tree -p vsh-bash --no-default-features --features worker -e features
```

Review the actual feature graph, not just Cargo.toml. Test runtime dependency directions and authority-bearing objects, not only package names.

## 7. P5 — runtime dispatcher and bounded Bash profile

### Touched areas

`crates/vbash/src/runtime.rs`, new narrow runtime execution module if needed, public `crates/vsh` exports/features, `vsh-bash` profile/adapter, integration tests under the owning crates.

### Work

- Introduce closed Monty/Bash request dispatch and explicit host enablement. Initialize only the requested backend; leave Monty startup and worker-path behavior intact.
- Translate Bashkit FS calls through the operation matrix. Include exists/missing observations, recursive child permissions, create-on-append, destination replacement and protected state directories.
- Enforce the selected synthetic namespace, unsupported operations and command profile through validated upstream seams. Reuse upstream parsing; no source-text authorization heuristics.
- Add `BashCall` effect origin without changing existing tags' meaning. Distinguish synthetic operations without letting them become workspace commit entries.
- Apply sticky policy/limit/profile failures independently of final shell exit code. Recoverable normal shell errors remain catchable. Treat a final nonzero as a typed noncommittable execution failure with bounded diagnostics.
- Detect incomplete/truncated execution data according to P0. Do not permit a final successful consumer to hide an upstream truncated producer.
- Keep program execution separate from commit: `RunMode::Auto` still passes every policy/hook/identity check, and a failed script cannot enter those actionable states.

### Acceptance workflows

1. Read-only shell observes files/directories and has no canonical mutation; `ALL_REQUESTS` can still review it.
2. Safe config rewrite previews, has expected diff, and commits only through normal host policy.
3. Recursive cleanup encounters a protected child and is denied even if shell catches the command error.
4. `printf changed > result; false` exposes failure diagnostics and never yields an approvable/committable handle.
5. `grep missing file || true` can complete when there is no independent policy/profile/limit failure.
6. `sed -i` preserves modes and records temporary effects without keeping a second writable overlay.
7. Missing/disabled/mismatched Bash backend errors before guest execution; no host-shell fallback.

Rust consumer fixtures must compile with default features, without default features where supported, and with `bash`; requests remain Monty by default.

## 8. P6 — artifact schema, identity and review integration

### Touched areas

`crates/vbash/src/artifact.rs`, `runtime.rs`, `hook.rs`, `review.rs`; `vsh-policy` transaction binding as needed; `vsh-store` only if persistence contracts genuinely require it; artifact/recovery tests.

### Work

1. Introduce one canonical tagged output envelope. Preserve raw Bash bytes; generate display text at boundaries instead of storing divergent copies. Apply output/diagnostic/artifact bounds before persistence and decode.
2. Bind the effective descriptor: language/backend/version/profile, namespace/env/cwd policy, feature/protocol fingerprint, limits and failure/output rules. Keep runtime-path/process/timing noise out of transaction identity.
3. Implement the next pending-artifact version with backend/output and new effect tags. Keep narrowly scoped V1/V2 decoding for existing data; no silent re-signing or backend inference from program text.
4. Bind and verify the new execution-evidence digest alongside existing binding validation. Cover the versioned result/output, parent effects, completion and completeness data; exclude eventual hook decisions, transaction ID and display choices. Recompute on artifact load. Corrupt mode/output/effect data cannot be used to influence a review while committing a different diff. Version the transaction identity rather than silently changing the old hash domain.
5. Extend `RequestEvent` with bounded execution context while retaining canonical diff, effects, policy and intent. Keep completeness/truncation truthful and each piece typed.
6. Preserve existing prepare/resolve/reservation semantics: one valid resolution, hook identity/TTL, approval/rejection/re-review, stale revalidation and commit recovery. No approval from a later run applies to an earlier descriptor.
7. Keep all JEV handler semantics and thresholds unchanged; only evidence routing gains Bash context. Untrusted shell text/output cannot become model instructions.

### Acceptance

- Identical code/intent under Monty and Bash cannot share approval identity.
- Changes to effective root/env/limit/profile/backend version invalidate reuse; changing display detail does not redefine the transaction.
- Round-trip binary stdout/stderr, mode effects and canonical evidence without loss.
- Restart preserves valid new previews and their review evidence. Old records are interpreted as their original format, or rejected explicitly when unverifiable.
- An older binary encountering a new artifact fails safely; no destructive automatic history migration.
- Pending review can be approved by a deterministic handler/JEV substitute; hard deny, invalid execution and stale state cannot.
- Cancellation or duplicate review resolution cannot race into double commit.

Re-run existing recovery/fault-injection tests; execution changes must not weaken the already durable commit pipeline.

## 9. P7 — native Python API, capability and tool integrations

### Touched areas

`crates/vsh-python/src/lib.rs`; `src/vsh/_native.pyi`, public exports, `hooks.py`, `pydantic_ai.py`, `_judge.py` only where evidence adaptation is needed; `src/vsh/mcp/native_tools.py`, `surface.py`, `prompts.py`, codemode/server; `src/vsh/cli.py`; corresponding tests.

### Work

- Bind `Language`, typed `BashConfig`/Bash-specific limits, `BashResult` and typed failure diagnostics. Use keyword-only Python options and update overloads; preserve current Monty conversion behavior.
- Add `bash=` host configuration and `language=` request selection to the existing preview/request workflow. Reject request-object plus conflicting direct kwargs consistently; do not add duplicate convenience families.
- Discover the bundled Bash worker only when Bash is enabled. Keep Monty `worker_path` separate. Resolve binaries from explicit configuration or trusted installed-package locations, not guest PATH.
- Route `HookedRuntime` through the same native backend and commit gate. Preserve `judge.hook_handler`, `review_instructions`, existing hook scope defaults and the capability constructor.
- Extend the existing `vsh_run` tool contract. Restrict advertised language choices to enabled backends and validate again in native runtime. Existing filesystem tools remain supported and use the same snapshot/policy semantics for their own requests.
- Propagate cancellation with an internal request-scoped native cancellation handle checked by the parent loop; no new speculative public cancellation API is required. Cancelling an async wrapper must not leave detached work that later auto-commits.
- Keep blocking native work off the Python event loop and release the GIL as in existing native execution. Test thread-safety rather than assuming PyO3 release alone implies cancellation or parallel safety.
- MCP/codemode should call the same language-aware run tool with the same host allowlist. Never expose the worker binary as a raw process tool. CLI output distinguishes stdout, stderr, exit code, policy state and noncommittable execution failure.

### Acceptance

Rust and Python agree on diff/decision/evidence for the same backend request. Both Python preview overloads work. Typed code passes Ruff/ty/basedpyright; `.pyi` is accurate. Capability tools include correct descriptions and schema choices. Deterministic agent tests prove normal commit, pending-review approval, feedback/re-review, deny and cancellation behavior; no live API calls run in CI.

Add executable, small examples rather than helper-heavy demonstrations:

- `examples/native/bash_workflow.py`: consolidated disposable binary preview/commit and deterministic strict-policy capability handler; negative cleanup/denial cases live in tests.
- `examples/native/bash_judge.py`: current `judge.hook_handler` API with independent agent/judge models and evidence-first configuration review; real calls explicit/optional, deterministic model substitute in tests.
- A Rust example under the existing public/runtime crate's `examples/` following current conventions.

Future checks (use current repo configuration, no temporary lenient config):

```sh
uv run ruff check
uv run ruff format --check
uv run ty check
uv run basedpyright
uv run pytest --cov=src/vsh --cov-branch --cov-report=term-missing --cov-fail-under=100
```

## 10. P8 — adversarial validation and performance optimization

Tests are added in their owning phases; P8 consolidates and audits them. It is not permission to postpone testing until the end.

### Security/semantic matrix

| Case family | Required assertion |
| --- | --- |
| `..`, repeated separators, absolute escape, NUL, invalid encoding, oversized path | No out-of-root access; typed bounded failure |
| Existing symlink, link loops, link in a parent component | No host or implicit guest traversal; explicit supported opaque read only |
| Protected child in recursive copy/remove/rename | Child authorization enforced; no commit after caught denial |
| stat/exists/list of protected or missing path | No disclosure-through-fabricated-success; correct dependency observation |
| Input, call, read/write, intermediate, output, node/effect and duration limits | Exhaustion sticky, bounded evidence, no approval of incomplete execution |
| Nested source/eval/subshell/function loops | No fresh budget, alternate FS or command-profile escape |
| Wrong protocol version/tag/ID/frame/body lengths, byte flood, child exit | Parent remains bounded; worker retired; no sealed success |
| Create→delete, modify→restore, rename→overwrite, sed temp files | Canonical final state correct; transient side effects preserved |
| chmod/no-op mode/unsafe bits/stale mode | Correct metadata diff, authorization and real commit behavior |
| Nonzero final exit with writes; handled ordinary nonzero | First noncommittable; second normal unless sticky violation |
| Binary output and intermediate truncation | Bytes preserved; no silent partial data commits |
| Worker reuse and concurrent independent runtimes | No env/cwd/function/output/FD/state cross-contamination |
| Cancel during simulation, review, preparation and host commit | Correct cancellation or recoverable commit result; never assume rollback |
| Artifact tampering, unknown versions, descriptor changes, old approvals | Fail closed and preserve recoverability |
| Hook/JEV approve/reject/re-review/error, `ALL_REQUESTS` reads | Existing authority respected; feedback reaches agent |
| Feature-off/missing worker/wrong worker/unsupported platform | Actionable error before execution; no fallback |

Use real public APIs and temporary filesystems where possible. Add codec fuzz/property tests for malformed framing/path inputs and randomized VFS operation sequences where they catch semantic bugs. Differential-test the deliberately supported command subset against host Bash **only in isolated test fixtures**, never by running arbitrary model/user source on host Bash. Record version/locale and expected documented divergences; host Bash is a compatibility reference, not authorization logic.

### Benchmark matrix

Reuse `crates/vbash/examples/native_benchmark.rs`, `benchmarks/native_pyo3.py`, `benchmarks/process_tree.py` and `benchmarks/compare.py` where their semantics fit. Add one focused Bash benchmark entrypoint if necessary, not a general benchmark subsystem.

Measure four layers separately: Bashkit + its in-memory VFS (reference), Bashkit + VSH gateway/RPC, full VSH preview, and full preview/review/commit without live network. Keep existing Monty native/PyO3 cases as regression baselines. The reference layer's faster numbers are not an equivalent-security product comparison.

Workloads: no-op; one/ten reads; twenty-file edit; metadata-heavy find/grep; sed atomic replacement; recursive copy/delete; 1K/10K small appends; large buffered pipelines; large lazy base snapshot with a small edit; denial/error paths; read-only and mutation hook paths; cold process and warm process/fresh interpreter; independent-runtime scaling. Use fixed fixtures and multiple payload sizes within known configured limits. Intentionally over-limit cases are correctness measurements, not throughput samples.

Collect p50/p95 (retain existing p99 where already reported), variance/spread, cold/warm split, per-stage latency, parent/child peak RSS, allocations where supported, real bytes materialized/hashed/stored, IPC calls/bytes, artifact/effect size and correctness/state checks. Keep raw samples and environment/commit/lock metadata. Report workload sample count and confidence/noise; a handful of averages is insufficient.

### Optimization order

1. Remove accidental extra snapshot/content copies or double accounting introduced by the integration.
2. Remove needless metadata round trips/materialization only with equivalent observations and authorization.
3. Measure append byte amplification and CAS growth. Consider bounded append buffering/content storage changes only if they retain every required intermediate observation/effect and crash/review semantics. Keep this a separately reviewable optimization, not an unbounded rewrite of storage.
4. Tune bounded process reuse/reset and buffer sizes; monitor retained memory across long runs.
5. Consider new batched primitives only when the measured IPC share justifies a larger protocol/atomicity surface. Shared memory and in-process Bash remain deferred unless existing gates cannot be met.

Do not optimize by disabling audit evidence, skipping checks, relaxing budgets, reducing fixture sizes or silently accepting Bashkit approximations. If a target cannot be met without weakening semantics, document the cost and revisit scope with the user.

### Coverage and review gate

Preserve current Python 100% line/branch gate and existing Rust aggregate floors (79% lines, 70% functions, 81% regions at the inspected baseline). Do not lower them or widen ignore patterns to hide the new backend.

New owned gateway/protocol/result/adapter logic targets full meaningful line and branch coverage. Close remaining branches with behavioral tests; any genuinely untestable/platform-specific exception requires explicit review and user acceptance, not a dummy test or blanket exclusion. Stable LLVM region coverage is not proof of branch coverage: report the distinction and add a supported branch-measurement job if needed rather than inventing a flag or claiming 100% branches from regions.

Audit the implementation against I1–I10 and the test matrix in a fresh review context before declaring merge readiness. Triage findings against actual code/evidence. Performance gates include paired Monty non-regression and an explicit measured Bash budget, not an unsupported speedup promise.

## 11. P9 — packaging, documentation and release-readiness

### Distribution/build work

- Update `scripts/vsh_build_backend.py` to build/stage/verify the exact matching workers for supported targets; keep cleanup and repeated builds reliable. Add the worker to the sdist build graph without accidentally linking its interpreter into the host feature set.
- Update `pyproject.toml`, `crates/vsh-python/Cargo.toml`, Rust feature forwarding and release metadata only as needed. Existing package names remain `vsh`, `vsh-runtime`, `vsh-python`, `import vsh` and compatibility `vbash`.
- Update `.github/workflows/ci.yml` and `publish.yml`, package/publish order and scripts under `release/`; new shared/adapter crates precede dependent runtime/facade packages. Check exact internal pins and external pins through existing version validation.
- Build host/wheels and workers in deliberately scoped Cargo commands. Verify feature unification does not activate RealFs/network/other interpreters or unexpectedly compile Bashkit into a Monty-only consumer.
- Validate cold installation from wheel and from sdist outside the repo, native worker discovery, executable permissions, target architecture, mismatch/missing-worker diagnostics and source-free Rust consumer smoke.
- Linux/macOS Bash support is advertised only with conformance evidence. Keep existing Monty support on other platforms; unsupported Bash must reject early and be documented. Do not fake permission semantics to make a platform matrix green.

### Documentation/examples

- Separate Rust and Python API documentation; label the supported Bash profile and platform matrix precisely.
- Guided tutorials: preview/commit; read-only shell; deterministic hook; JEV review with canonical evidence; MCP/codemode; failures/denials/stale/partial writes; resource limits and binary output.
- Explain why familiar commands/flags may be unsupported, why pipelines are buffered, why `parallel` is not available, and why VSH is not a host command/container sandbox.
- Publish measured benchmarks with method, fixtures, versions and limits. Keep historical results clearly identified; no stale playground output promoted as current performance.
- Update capability instructions/tool descriptions, README package summaries, changelog and migration notes for intentional Rust result changes.
- Preserve the existing documentation design and copy-as-Markdown/dark-light behavior; no UI redesign in this feature.
- Regenerate/check `llms.txt` and `llms-full.txt`. Keep Pages build docs-only and independent of Rust compilation.

Existing final-check commands (after dependencies are intentionally updated/locked):

```sh
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo llvm-cov --workspace --all-features --all-targets --locked --summary-only --ignore-filename-regex '(vsh-python|vsh-worker)' --fail-under-lines 79 --fail-under-functions 70 --fail-under-regions 81
cargo audit --file Cargo.lock
cargo deny --all-features --locked check
uv run ruff check
uv run ruff format --check
uv run ty check
uv run basedpyright
uv run pytest --cov=src/vsh --cov-branch --cov-report=term-missing --cov-fail-under=100
uv build
python3 release/check_versions.py
python3 scripts/generate_llms_txt.py --check
uvx --from zensical==0.0.57 zensical build --clean --strict
python3 scripts/check_docs.py
```

Revalidate these commands against then-current CI; preserve stricter gates if they have changed. Add explicit Bash host/worker feature-matrix jobs and new-code coverage gates alongside this existing aggregate gate. Do not let an existing worker ignore pattern accidentally exclude new security code. Run the existing rustdoc and release consumer/wheel/sdist smoke checks too; their complete target-specific commands live in CI/release scripts.

Dependency safety is time-dependent. The prior isolated 0.18.2 graph had 125 registry packages and no reported RustSec findings at database revision `9b3a3b73a7f42606494c943e95f8196e9994df46`. That finding is not a permanent guarantee or a release waiver. Re-run audit/license/feature checks on the actual final graph and binaries; no global advisory ignore to force a release.

## 12. Completion checklist and handoff

- [x] P0 feasibility evidence complete for the explicitly bounded profile; no host fallback or silent truncation.
- [x] P1 semantic/API/wire contracts implemented; Unix Bash requirement and Windows rejection are explicit.
- [ ] P2 gateway semantic tests pass; the proposed 5% all-workload performance gate remains open.
- [x] P3 modes and atomic edits have real commit/review/stale coverage.
- [x] P4 worker/protocol/resource/cancellation/pool boundaries pass local adversarial tests.
- [x] P5 supported Bash workflows and failure semantics work through public Rust runtime.
- [x] P6 durable artifacts, identity, upgrade/recovery and hook/JEV evidence pass.
- [x] P7 Python, capability, MCP/codemode and CLI are typed and behavior-tested.
- [ ] P8 security review and coverage floors pass locally; performance and hosted matrix closure remain open.
- [ ] P9 local macOS wheel/sdist/install/docs verification implemented; hosted Linux/Windows gates remain unexecuted.
- [x] New surfaces have no placeholders, stale aliases or additional coverage/advisory waivers; pre-existing user changes are preserved.
- [x] Remaining gates are documented, not waived; no universal merge/release-readiness claim is made.

After each phase, record: objective, changed paths, actual commands/results, artifact paths, benchmark differences if relevant, reviewed findings, decisions changed, blockers and the next safe step. Completed checkboxes require evidence, not elapsed work.

Stop conditions: required safety semantics cannot be enforced with the reviewed dependency; a shared-gateway refactor changes Monty behavior unexpectedly; complete evidence cannot be bound; child/parent resource usage is unbounded; async cancellation can cause unobserved commit; packaging enables host capabilities; or measured regression cannot be resolved within scope. Exhaust safe narrow alternatives, update the plan, and request direction instead of expanding into a new shell/OS implementation.

The final implementation handoff should identify the exact supported profile, known limitations, test/coverage results, measured latency/memory/build costs and artifact readiness. **Publishing remains a separate user-authorized action.**
