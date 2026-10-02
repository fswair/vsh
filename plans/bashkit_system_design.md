# Bash execution on VSH — system design

Status: opt-in public Rust/Python Bash integration, capability/MCP/CLI dispatch, tagged byte-authoritative V3 artifacts, hooks, cancellation and parent-evidence bounds are implemented in this checkout. The bounded profile is revision 4. Final verification and platform publication gates are tracked in the [completion evidence](bashkit_p7_p9_evidence.md).
Updated: 2026-10-01.
Baseline: VSH `8550facbe94485cb12bfe007fbb3575a168b6afc`; Bashkit `0.18.2` at `567511386572d4c4b9f649ab1da97aad53b9f341`.
Companion: [implementation plan](bashkit_implementation.md).
Implementation evidence: [gateway checkpoint](bashkit_p2_evidence.md), [mode/worker checkpoint](bashkit_p3_p4_evidence.md), [historical evidence/artifact checkpoint](bashkit_p6_evidence.md), [public integration and final gates](bashkit_p7_p9_evidence.md).

This document records the implemented design and its original acceptance criteria. The Bash APIs below are available in this checkout, not in the already published 0.5.0 release. Implementation is authorized; publication is not. No new release version is assigned here. Read the implementation plan and phase evidence for measured results and platform limitations.

## 1. Product objective

Let a person or agent use familiar Bash commands to inspect and modify a workspace **virtually**, inspect the actual consequences, and commit only the approved transaction. Rust uses native crates; Python uses the same Rust implementation through PyO3. Existing Monty workflows remain available.

Bashkit owns parsing and shell/builtin execution. VSH owns filesystem authority, observation dependencies, resource accounting at its boundary, canonical changes, policy, review, durable approval, revalidation, and host commit. We are embedding an execution frontend, not translating shell text into a guessed list of VSH tools.

The first convincing workflow is a repository-maintenance script using `find`, `grep`, `sed -i`, `cp`, `mv`, and `rm`: preview it, show exact changes and effects, allow a deterministic hook or JEV to review it, and commit without rerunning the script. A second workflow reads and transforms data without mutations and still produces reviewable access evidence.

Success means useful Bash coverage without weakening VSH guarantees or imposing Bashkit startup/dependency overhead on Rust consumers that do not enable Bash. Performance claims require measurements, not comparison of project feature lists.

## 2. Scope and non-goals

### First integration

- Optional, exact-pinned Bash backend, exposed through the existing runtime request/preview/commit workflow.
- Fresh interpreter state per transaction; process reuse only after a verified reset.
- Workspace-rooted regular files/directories; explicit synthetic namespace described below.
- Content reads/writes, append, directory operations, copy, rename, removal, and bounded permission-mode changes needed by atomic edits.
- Parent-owned `VirtualFs`, common policy-aware gateway, typed worker requests, bounded evidence/output.
- Existing Rust/Python hooks, Pydantic AI capability, JEV, MCP and codemode integration through the same commit gate.
- Contract tests, adversarial tests, reproducible latency/memory benchmarks, packaging and documentation.

### Explicitly excluded

- A new Bash parser, shell-to-Python transpiler, or reimplementation of coreutils.
- Host Bash, arbitrary executables, host filesystem mounts, HTTP/SSH, native process spawning by guest code, Bashkit Python/TypeScript/SQLite runtimes.
- A general Virtual OS, container replacement, multi-tenant SaaS isolation claim, job scheduler, or persistent shell session.
- Custom tool registration; that remains a separate [deferred proposal](custom_virtualfs_tools.md).
- Full POSIX/GNU compatibility, interactive shells, terminal emulation, real parallel jobs, streaming process pipelines.
- Symlink creation/traversal, hard links, ownership changes, special files, timestamp mutation, and general `/tmp` support in the first profile.
- A new LLM judge, new JEV thresholds, an `under_review` state, or an automatic approval bypass for shell requests.
- An in-process Bash mode in the initial release. It can be considered after measuring the subprocess boundary; it must never be a silent fallback.

## 3. Evidence and current system

Sources are evidence, not authority over VSH's product/security contract.

| Current fact | Design consequence |
| --- | --- |
| Requests have `Language`; receipts have tagged `ExecutionOutput` | Keep a closed, typed execution boundary, not a plugin registry |
| Monty subprocess sends OS/tool calls to the parent; the parent owns `&mut VirtualFs` | Keep filesystem authority in the parent for Bash too; no snapshot export/import loop |
| `vsh-execution` owns shared authorization, `VirtualRoot`, and filesystem budget accounting | Both adapters use the same parent-owned gateway |
| `VirtualFs` records effects/read-set/write-set and has `with_effect_origin` | Reuse these; do not infer evidence by parsing stdout or comparing a separate Bashkit VFS |
| VFS has bounded mode-setting and opaque link reads; no symlink-create primitive | Opaque links do not imply traversal support |
| New pending artifacts use `VSHPND03`; legacy readers remain narrowly scoped | Backend/output/evidence are sealed; old data must not silently acquire new meaning |

Local source map: [runtime](../crates/vbash/src/runtime.rs), [artifacts](../crates/vbash/src/artifact.rs), [review](../crates/vbash/src/review.rs), [hooks](../crates/vbash/src/hook.rs), [Monty adapter](../crates/vsh-monty/src/lib.rs), [Monty worker client](../crates/vsh-monty/src/worker.rs), [worker child](../crates/vsh-worker/src/child.rs), [VFS](../crates/vsh-vfs/src/lib.rs), [policy](../crates/vsh-policy/src/lib.rs).

### Upstream findings that constrain this plan

1. Single-interpreter background jobs and `xargs -P` run sequentially. `parallel` is a dry-run stub, not a parallel executor. Do not claim scheduler races that do not exist in this version. [Jobs](https://github.com/everruns/bashkit/blob/v0.18.2/crates/bashkit/src/interpreter/jobs.rs), [xargs](https://github.com/everruns/bashkit/blob/v0.18.2/crates/bashkit/src/builtins/pipeline.rs), [parallel](https://github.com/everruns/bashkit/blob/v0.18.2/crates/bashkit/src/builtins/parallel.rs).
2. Pipelines pass buffered stage output to the next stage sequentially. This is a resource/compatibility constraint, not a streaming OS pipe. [Interpreter](https://github.com/everruns/bashkit/blob/v0.18.2/crates/bashkit/src/interpreter/mod.rs#L5081).
3. Custom filesystems own their quotas. Default extension methods report zero usage/unlimited capacity. Bashkit profile selection does not enforce our filesystem budgets. [Builder](https://github.com/everruns/bashkit/blob/v0.18.2/crates/bashkit/src/lib.rs#L1929), [traits](https://github.com/everruns/bashkit/blob/v0.18.2/crates/bashkit/src/fs/traits.rs).
4. Text/lazy mounts can wrap a custom filesystem in a separate writable overlay. Do not enable these helpers: all workspace writes must reach VSH. [Builder layers](https://github.com/everruns/bashkit/blob/v0.18.2/crates/bashkit/src/lib.rs#L3276).
5. `sed -i` writes a sibling temporary file, copies the original mode using `chmod`, then renames over the target. Rejecting every chmod would break an important first-release use case. [Atomic replacement](https://github.com/everruns/bashkit/blob/v0.18.2/crates/bashkit/src/builtins/atomic_write.rs).
6. Fresh shell state is not full reproducibility: `$RANDOM`, `shuf`, time-related commands and temporary-name generation need a defined profile. Fixing `date` does not fix every source of nondeterminism. [Interpreter](https://github.com/everruns/bashkit/blob/v0.18.2/crates/bashkit/src/interpreter/mod.rs), [shuf](https://github.com/everruns/bashkit/blob/v0.18.2/crates/bashkit/src/builtins/shuf.rs).

The prior isolated smoke run observed successful pipeline filtering, `sed -i`, sequential background/xargs append, `parallel` producing no file, and `write; false` leaving a changed virtual file with exit code 1. These are upstream observations, **not VSH integration tests or performance results**. Reproduce them in Phase 0; do not depend on temporary research files.

## 4. Security and correctness invariants

I1. Only the host committer writes the real workspace. Guest filesystem calls affect the transaction overlay only.

I2. The parent owns the only authoritative workspace snapshot, overlay, effect ledger, read-set, write-set, policy and filesystem budget. Worker claims about those objects are never trusted.

I3. Every observable read and mutation crosses the gateway, including metadata/existence checks, recursive children, atomic-edit temporary files, and failed access attempts. Policy denial cannot be erased by `|| true`, a subshell, or a worker's successful completion message.

I4. Review and commit refer to the sealed execution result and canonical diff. Commit never re-executes Bash. Receipt detail/rendering choices do not change the transaction's meaning.

I5. Policy hard deny, invalid/incomplete execution, resource exhaustion, malformed protocol and stale dependencies cannot be converted to approval by a hook, JEV, or the agent.

I6. A valid policy-review transaction may be approved by the configured handler under the existing authority/TTL rules. `HookScope.REVIEW_REQUIRED` remains the default; `ALL_REQUESTS` also checks auto-approved actionable requests. Read-only actionable requests remain in scope according to the existing hook rules.

I7. Namespace, backend/version/profile, effective limits and host-provided execution inputs are identity-bearing. Monty and Bash executions of identical text are different identity domains.

I8. Bounds apply before allocation/deserialization/mutation where possible. A truncated display is marked; truncated execution data must not silently become a successful program input or output file.

I9. Each transaction has one execution owner. No shared interpreter or writable filesystem between simultaneous transactions; cancellation, worker death and pool reuse cannot leak state.

I10. No fallback to host shell, weaker limits, default Bashkit VFS, or a different backend when configuration is invalid or a worker is missing.

## 5. Architecture and crate ownership

```text
Rust API / PyO3 / HookedRuntime / Pydantic AI / MCP
                          │
                     VSH Runtime
                          │
                 per-request dispatcher
                   ┌──────┴───────┐
                   │              │
              Monty adapter   Bash adapter ─── bounded IPC ─── Bashkit worker
                   │              │                            Arc<RpcFilesystem>
                   └──────┬───────┘                                  │
                          │                     FS requests ──────────┘
                  FsGateway<'transaction>
            authorization / paths / budgets / origin
                          │
                  parent-owned VirtualFs
                          │
             diff + effects + dependencies + identity
                          │
             existing policy / hook / JEV / commit
```

### Implemented module split

| Area | Responsibility | Must not own |
| --- | --- | --- |
| New `crates/vsh-execution` | Shared gateway, virtual-root mapping, common filesystem accounting/statistics/errors | Bash parser, Monty values, transaction store, approvals |
| Existing `crates/vsh-monty` | Monty value conversion and call/tool dispatch through the gateway | A second implementation of common authorization/budget rules |
| New `crates/vsh-bash` | Parent client, bounded wire contract, Bash-specific mapping; optional `worker` feature and `vsh-bash-worker` binary | Commit authority, a second canonical filesystem, generic plugins |
| Existing `crates/vbash` (`vsh-runtime`) | Closed language dispatch, tagged result, artifact binding, review and commit lifecycle | Shell semantics or builtin implementations |
| Existing `crates/vsh` | Public Rust exports and `bash` feature forwarding | Duplicate runtime logic |
| Existing `crates/vsh-python` / `src/vsh` | Typed conversions, worker discovery, existing SDK/integration surfaces | Python simulation or filesystem policy implementation |

Use two new packages, not a crate for every conceptual box. Keep wire types private to `vsh-bash` unless a genuine second consumer requires otherwise. Gate parent and worker modules so the `vsh-bash-worker` build does not need runtime/commit/store authority dependencies. A dependency on a type is not an authority grant, but avoiding unnecessary guest dependencies also reduces build cost.

Feature arrangement: `vsh-bash` defaults to `host`; `worker` enables the interpreter, its Tokio runtime and the guest entrypoint; the binary requires `worker`. A worker-only build uses `--no-default-features --features worker`. The normal host feature does not depend on Bashkit. `vsh` and `vsh-runtime` expose an opt-in `bash` feature, disabled by default. Verify the actual resolved feature graph in each shipped artifact; Cargo feature unification can otherwise defeat this separation.

Do not change Monty's upstream protocol or turn both workers into one universal guest protocol in this pass. Shared process-management helpers can be extracted only when real duplication is demonstrated and Monty performance remains unchanged.

## 6. Ownership, worker lifecycle and IPC

### Why the filesystem stays in the parent

The initial suggestion of `Arc<Mutex<VirtualFs>>` inside the Bash worker is unnecessary for the existing architecture. The worker's `Arc<dyn FileSystem>` will contain an RPC proxy, not `VirtualFs`. Async method calls become bounded requests to the parent, which already has exclusive mutable access to the transaction filesystem.

This avoids copying the snapshot, importing an untrusted final diff, reproducing policy inside the guest, and locking the core VFS globally. The remaining cost is IPC per filesystem request; it must be measured explicitly.

### Lifecycle

1. Validate language/config/source/budgets before snapshot work where possible.
2. Snapshot through the existing VSH path and borrow the resulting VFS into a gateway.
3. Acquire a compatible idle Bash worker or spawn an approved binary with sanitized environment, non-workspace cwd, controlled descriptors and bounded stderr.
4. Check protocol version, VSH worker version, Bashkit version, feature/profile fingerprint and supported limit behavior. A configured executable is trusted host configuration; a self-reported handshake does not authenticate arbitrary binaries.
5. Configure effective limits and session ID; construct a new Bash object with fixed host-selected env/cwd and RPC filesystem.
6. Run source. The synchronous parent services FS requests and accumulates authoritative evidence until completion or failure.
7. On success, validate terminal outcome, flush output, obtain reset acknowledgement, then return the clean process to its pool. Drop the Bash object, request references and guest session state first.
8. On timeout, cancellation, protocol error, allocation-limit exit, panic or failed reset, kill/reap the process and discard it from the pool. Never reuse a tainted worker.
9. Evaluate and seal the transaction only after the execution checks below pass.

Process reuse does not mean interpreter reuse. Maintain a separate Bash pool, reuse the current idle-worker cap convention, and retire workers after bounded reuse or abnormal retained-memory growth. Add no background worker when only Monty is used.

### Wire contract

Use a small versioned, length-prefixed binary protocol. Preferred codec: the repo's pinned `postcard` with explicitly pinned serialization dependencies; pin/scan any new direct dependency at implementation time. Do not deserialize arbitrary Monty objects for Bash.

Messages: handshake, configure, execute, filesystem request/reply, bounded stdout/stderr chunk, finish, reset/reset-ack and fatal error. Each carries or is tied to a session ID; filesystem calls have monotonically increasing request IDs. Permit only one outstanding FS operation per session initially. Reject wrong-session replies, duplicate/out-of-order messages, operations after finish and unexpected tags.

Bound frame length before reading the body. Bound nested lengths/counts before allocating decoded strings/vectors; an outer frame cap alone does not make an unbounded decoder safe. Reject trailing bytes, invalid UTF-8 paths, NUL paths, numeric overflow, unsupported flags and too many directory entries. Use existing VSH value/path/output limits to derive frame limits; never trust a worker-supplied bound.

Transport stdout is protocol only. Guest stdout/stderr are data; process stderr is bounded diagnostics. Drain all channels without deadlock. Apply a parent-owned wall deadline to configuration, execution and reset; filesystem traversal loops in the parent must also check deadline and operation budgets.

The guest can use a single-thread Tokio runtime. An RPC exchange may serialize access to its duplex transport, but must not retain a lock across `.await` or require the blocked interpreter task to service its own reply. No nested Tokio runtime is introduced into the host's `Runtime::run`.

### Memory and threat boundary

Explore reusing the already pinned `monty-alloc = "=0.0.22"` in the **Bash worker binary only**. Its generic global allocator and `set_limit(..., false)` enforce a live Rust-allocation ceiling independently of the Monty interpreter. Existing code uses the same facility in `vsh-monty-worker`. This is an implementation reuse, not a second Python runtime.

Phase 0 must prove suitability: process-baseline accounting, reset behavior, allocation-limit termination, Tokio allocations and exit classification. The inspected allocator has a 4 MiB hard-limit headroom beyond the configured session budget and process baseline; document and bind the effective policy rather than calling it an exact RSS cap. Stacks, non-Rust allocations and kernel memory require separate consideration. No custom unsafe allocator should be written for this feature.

If that approach cannot provide the required budget contract, stop and resolve the limit implementation; do not silently treat Bashkit's cooperative limits as equivalent. Bound parent allocations separately. A subprocess provides crash/timeout containment, **not an OS security sandbox against arbitrary native-code compromise**. Hostile multi-tenant deployments still require OS-level isolation; this integration must not market itself as an E2B/VM replacement.

## 7. Filesystem gateway contract

The gateway borrows `&mut VirtualFs` plus immutable policy/root configuration and transaction-local accounting/evidence. It exposes operations, not a raw mutable-VFS escape hatch. Backend conversion code cannot emit synthetic successful effects on its own.

Order for each call: validate request → normalize/map path → determine authorization scope → authorize → preflight/charge bounded work → perform VFS operation → record result/effects/observations → return typed reply. Denial/limit state is sticky across shell error recovery. Ordinary `NotFound`, `AlreadyExists` and similar recoverable filesystem errors remain distinguishable from denial.

Keep existing `CallPolicy` in `vsh-policy`. Extract the existing rules, not a new competing policy language. Preserve Monty behavior in characterization tests before refactoring; any discovered difference in listing/recursive authorization requires an explicit plan correction rather than an incidental semantic change.

| Bash filesystem operation | Gateway obligations |
| --- | --- |
| read_file | `ContentRead`, byte limit, content observation, binary bytes |
| stat / exists | `MetadataRead`, including missing-path observation; denied is not fabricated as absent |
| read_dir | `DirectoryRead`, bounded entries and required child visibility checks; no protected-name disclosure |
| write_file | Create/Modify policy, parent/type/size checks, byte/effect accounting, precondition |
| append_file | Preserve existing VSH read/write dependencies; charge content reconstruction work as well as appended bytes; create-if-absent behavior explicit |
| mkdir | Create policy for every missing ancestor when recursive; preserve existing parents |
| remove | Delete policy for every affected child; fail closed before a forbidden subtree can be committed |
| rename | Source and destination policy for every affected rebased child, replacement destination preconditions; handle type/ancestor conflicts |
| copy | Source read and destination create/modify policy; bounded traversal, modes, destination replacement and no recursive self-copy |
| chmod | New semantic mode-change primitive, Modify authorization, mode validation, metadata evidence and commit checks |
| read_link | Opaque link target only, bounded reply and observation; never follows it |
| symlink / mkfifo / set_modified_time | Typed unsupported outcome; no no-op success |
| extension usage / limits | Truthful scoped statistics/capacity, not default zero/unlimited; refuse dependent commands when the trait cannot express a required authorization failure |
| extension snapshot / restore | No guest-owned workspace snapshot or restore; unsupported, never an alternate mutation path |

Authorize all affected paths of a compound primitive before applying that primitive where feasible. Budget preflight must itself be bounded. A script can have earlier successful virtual writes before a later error; the execution/commit rules, not an invented global rollback inside each builtin, determine whether those changes are eligible.

Internal/synthetic accesses count toward operation and memory limits too. Request counters are charged on cache hits and failed requests; memoization cannot bypass budgets or erase observations. Workspace-private VSH artifacts remain inaccessible through traversal, globbing, direct reads, rename and temporary-file names.

Maintain separate monotonic counters for total read/write/materialization work and live gauges for overlay bytes/node growth. Deleting a file can release live capacity but never refund work already consumed. Base-snapshot limits, per-file limits, live overlay growth, transient evidence/blob growth and IPC memory are distinct quantities; a large lazy base is not automatically a large resident allocation. Update cached usage by operation deltas, with checked arithmetic and recomputation checks in tests, rather than walking the whole snapshot on every `df`/write. Define whether a capacity limit includes the base explicitly; no silent reinterpretation as request-only growth.

### Mode support

Add `VirtualFs::set_mode` (working name) for files/directories with a semantic metadata effect, existing read/write preconditions, and canonical `MetadataChange` behavior. Preserve node type/content and avoid reading/re-hashing file bytes for a mode-only operation where current storage permits it.

Allow only the explicitly supported permission-bit mask; reject setuid/setgid/sticky or ownership semantics unless separately designed. First-profile mode changes target Unix permission semantics. Windows behavior is a platform gate: never claim meaningful chmod semantics by returning success for unsupported host behavior. Preserve original mode during atomic content replacement; test this through actual commit, not only diff generation.

An unchanged mode may be a filesystem no-op, but authorization/accounting and any observation it entails still happen. The new effect variant must be encoded, rendered and reviewed everywhere existing effects are handled. Existing policy cannot approve a mode change merely because file bytes are identical.

## 8. Initial compatibility profile

Profile ID: `vsh-bash-bounded-v4`, worker identity revision 4. Immutable semantics once released; a security-meaningful change creates a new profile fingerprint and matching worker identity. This is a compatibility/safety profile, not a separate product.

### Namespace and environment

- Workspace mount and initial cwd use the configured virtual root (default `/workspace`); no real host prefix is supplied to the guest.
- A minimal synthetic `/` can expose the mount and explicitly supported devices only. Synthetic directory metadata is fixed, accounted and noncommittable.
- Support `/dev/null` as an interpreter-owned sink/empty source: the upstream redirection engine bypasses `FileSystem` for this device, as asserted by the contract probe. These redirects are bounded by interpreter work/intermediate/allocation limits and the parent deadline, not filesystem byte/entry counters. Actual synthetic FS RPC accesses remain accounted separately. Do not fabricate workspace effects or claim that every null-device redirect is visible to the gateway. No other `/dev` mapping is implied.
- `$HOME` is the virtual root. Use fixed locale/timezone and curated shell identity; do not inherit host environment, secrets, PATH, `BASH_ENV`, `ENV` or startup files.
- `/tmp`, `/home/...`, `/proc`, `/dev/fd`, FIFO and process-substitution filesystem access are unsupported initially. Explicit temporary files inside the workspace are normal VSH effects and may appear in evidence even when absent from the final diff.
- Reject mount escape, invalid encodings and symlink traversal. Lexically collapsing `..` is not a proof of safe resolution through a symlink.
- No general environment/cwd injection is exposed to model-facing tools. Host configuration additions must be bounded and identity-bearing if introduced later.

Reviewed profile correction: accept Bashkit 0.18.2's synchronous virtual `coproc`. Its body is awaited in the same interpreter; buffers and IDs are virtual, without native processes or file descriptors. Nested forms retain the same VSH gateway and policy. This is not POSIX concurrent coprocess execution. Coprocess buffers are constrained by worker allocation/work limits; do not claim that the upstream descriptor limit counts these buffers. `before_exec` is not a nested syntax validator. Process substitution may execute a virtual producer, but attempting to use its `/dev/fd` path is a sticky terminal namespace failure, even under `eval`, `source` or `sh`. Do not claim syntax-level rejection of an unused substituted path. No second parser, shell regex authorizer or Bashkit fork is introduced.

The dispatch guard rejects network builtins and successful-but-unimplemented stubs: `parallel`, `df`, `curl`, `wget`, `http`, `chown`, `kill`, `retry`, `watch`, `fc`, `env`, and `tar`. Entire `env` and `tar` commands are excluded initially: the former does not faithfully dispatch arbitrary commands, and the latter silently skips unsupported archive members. `cp`/`mv` accept plain operands and `--` (plus help/version), not ignored preservation, link, ownership, no-clobber or backup requests. Recursive `cp` is also excluded: the upstream copy RPC does not carry its recursion flag. `rm` additionally accepts implemented `r`/`R`/`f` and `--recursive`/`--force`, but not ignored interactive options. `chmod` accepts ordinary numeric/symbolic modes and explicit paths; symbolic special bits and dash-leading target operands are rejected (use `./name` for a filename beginning with a dash). These checks inspect already parsed argv, including nested/wrapped dispatch, not shell source text.

### Command behavior

Target regular-file workflows, normal redirection, sequential pipelines, conditionals, loops, functions and workspace shell scripts. Explicitly test `cat`, `printf`, `grep`, `sed -i`, `find`, `sort`, `head`, `tail`, `wc`, `cp`, `mv`, `rm`, `mkdir` and the tested flags; this list is a test commitment, not a claim of complete GNU compatibility.

`&`, `wait` and `xargs -P` may retain Bashkit's sequential semantics when tests pass; document lack of real parallelism. Disable/replace the misleading `parallel` stub with an unsupported-command error in the VSH profile. `sh`/`bash` guest invocations, `source`, functions and `eval` must remain in the same interpreter budget and filesystem authority. They never launch the host executable.

Unknown/disabled commands and unsupported filesystem features are profile failures, not successful stubs. Check builtin dispatch, functions, aliases, nested shells, `command`, executable workspace scripts and resolver paths; a superficial string/regex denylist is insufficient. A trusted builtin is not a license to skip filesystem authorization.

If upstream interception cannot reliably enforce the profile or report an unsupported operation, that is a Phase 0 blocker. Prefer an upstream fix followed by a reviewed exact-pin update; do not silently fork Bashkit or implement a competing shell.

### Byte-preserving output and narrow text commands

`cat`, ordinary copying and raw output retain bytes. Bashkit's shell variables and some text builtins use strings; this is not full POSIX binary-string compatibility. Resolved `printf` argv is restricted to literal text, `%s` and `%%` without width or precision; `%c`, numeric conversions and `-v` are terminal profile failures. `echo`/`printf` arguments containing numeric byte escapes are also rejected. These checks apply to nested and wrapped dispatch. VSH does not replace Bashkit's parser or formatter to broaden the profile.

### Timestamps and nondeterminism

Use fixed synthetic timestamps for metadata and explicitly state that timestamps are not canonical workspace state. `touch`, timestamp comparisons/preservation and related flags are unsupported until their semantics are deliberately supported. This includes new-file `touch`: upstream also requests a timestamp mutation after creation, so the request fails terminally. Create new files with supported content/redirection operations instead; do not pretend a timestamp was changed.

The initial contract is deterministic **commit of a concrete evaluated diff**, not universal reproducibility of every Bash command. Enumerate nondeterministic surfaces in Phase 0. Use a fixed virtual clock where supported; do not claim a fixed `$RANDOM` assignment also controls `shuf`, temporary names or all time values. Nondeterministic inputs that remain supported must be disclosed and the actual resulting diff/evidence sealed. Exact replay/seed APIs are deferred; cannot ship a “fully deterministic shell” claim.

## 9. Execution outcomes and transaction lifecycle

Use a language-neutral envelope with backend-specific value types and binary-safe output. A closed enum for two known frontends is enough; no public `Executor` plugin protocol yet.

| Outcome | Runtime behavior | Can a hook make it committable? |
| --- | --- | --- |
| Completed, final exit 0, no sticky violation | Canonical diff → normal policy → existing transaction state | Yes, if existing review rules allow |
| Ordinary inner command nonzero, explicitly handled, final exit 0 | Normal shell semantics; still reject any independent sticky violation | Same as above |
| Final nonzero exit, including partial virtual writes | Typed execution failure with bounded stdout/stderr/exit code and diagnostic change summary; no actionable approval/commit handle | No |
| Parse error / unsupported profile operation | Typed execution failure; no commit | No |
| Policy-denied access, even caught by shell | Preserve denial evidence; hard-denied transaction when execution evidence is complete, otherwise execution failure | No |
| Limit hit / internal truncation / cancelled / worker or protocol failure | Fail closed; no seal of incomplete evidence and no auto-commit | No |
| Host workspace changed after valid preview | Existing stale/revalidation outcome | No override of stale checks |

Do not add new public transaction states for Bash. Reuse the existing error and transaction-state model; add precise error variants only where callers need distinct handling. Partial diagnostic diff is not an approvable proposal. If failure occurs before complete evidence exists, mark diagnostic incompleteness and do not manufacture a canonical transaction from a best-effort worker response.

Output is byte-authoritative. Shell stdout/stderr may contain non-UTF-8 data; retain bounded bytes. UTF-8 replacement/escaped text is a display projection only. Output limits must not silently truncate intermediate data that later becomes a committed file. Phase 0 must test propagation through nested commands/pipelines, not just final `ExecResult` flags.

Cancellation before commit stops simulation/review and retires the active worker; a Python `to_thread` wrapper alone does not cancel native work. Wire explicit cancellation into the synchronous parent loop. Once host commit has entered its durable commit/recovery protocol, cancellation is not a promise of rollback: finish/reconcile through the existing transaction store, return an outcome that can be queried, and never automatically retry a potentially completed commit.

## 10. Identity, persistence and evidence

### Execution descriptor

Bind a versioned descriptor into `RuntimeConfigDigest` (or an explicitly versioned successor): language, exact backend/version, worker protocol/build features, compatibility profile, virtual-root/cwd/env policy, effective guest and gateway limits, clock/randomness policy, output/error policy, and execution mode. Keep dynamic process IDs, timing samples, diagnostic paths and receipt detail out of semantic identity.

Existing identity already includes program, snapshot/dependencies, policy and intent. Reuse that mechanism; do not substitute “same intent” or “same shell text” for the evaluated transaction. For the new identity version, also bind an execution-evidence digest covering the versioned result envelope, byte-authoritative output, parent-recorded effects, terminal status and evidence completeness. Construct it before the transaction ID; exclude the transaction ID itself, later hook decisions, display projections and timing data to avoid circular or presentation-dependent identity. Hash within existing bounds without copying the full payload again.

Recompute this digest on artifact load. A review must never display changed bytes/effects alongside an otherwise valid approval. This intentionally changes identity for new Monty records too; old records are interpreted only under their original version and trust rules, not silently upgraded to the stronger binding. Add a stable digest type in `vsh-types` only when implementing this actual binding, not as an unused public placeholder.

The host-configured worker executable is a trusted deployment component. Version/fingerprint verification and package provenance prevent accidental mismatches; handshake strings alone are not proof against a malicious replacement binary. Record the trust assumption.

### Artifact schema

Introduce a new pending-artifact version for tagged execution result, byte output, Bash origin, metadata effects and execution descriptor/evidence fields. Apply total, per-field and per-entry bounds before decoding. Unknown tags/versions fail closed.

Retain a narrowly scoped reader for existing V1/V2 durable artifacts only as required for safe upgrade/recovery. Interpret them as legacy Monty records; never relabel them Bash or silently recompute their identity. If an old approval cannot be validated under its original descriptor, require a fresh preview/review. Do not migrate or delete user transaction history automatically. This storage concern does not authorize restoring stale public APIs.

Rollback to an older binary must reject unknown new artifacts without corrupting existing records. Document upgrade/rollback behavior and test recovery of already-started commits separately from execution-backend availability.

### Review payload and JEV

Add language/profile/completion/output metadata to the existing `RequestEvent` evidence with bounded projections. Preserve canonical diff, actual effects including reads, read/write sets, policy decision, risk metrics, intent and existing content-evidence completeness/truncation flags. Atomic-edit temporary effects must not disappear just because the final diff is small.

Raw code, output, paths, intent and file content are untrusted evidence, not judge instructions. Reuse the current evidence-first prompting and `review_instructions` extension contract. Keep handler construction as `judge.hook_handler`; keep the current JEV default/threshold behavior unchanged.

The judge may approve a valid pending review, reject it, or request another review through existing feedback semantics. It cannot turn unsupported execution, missing critical evidence, policy hard deny or stale state into permission. Stdout does not prove that an operation succeeded; canonical state and observed effects remain primary evidence alongside request details and intent.

## 11. Implemented Rust and Python surfaces

These names are implemented and covered by compile/type and behavior fixtures. Installation from the published 0.5.0 release does not provide the new Bash backend yet.

- Add `Language::{Monty, Bash}`; existing requests default to Monty.
- Add `RunRequest::with_language` / Python keyword-only `language`.
- Enable Bash explicitly with a typed `BashConfig` on `RuntimeConfig` / `Runtime.open`. No config means Bash requests fail before execution. `worker_path` in existing constructors remains the Monty override; `BashConfig.worker_path` is the Bash override.
- `BashConfig` owns host-selected worker/limit options, not arbitrary Bashkit builder access or a mutable capability dictionary. Start with the one reviewed profile; no public profile registry.
- Keep `Runtime.preview(request)` and direct-argument preview overload. No redundant `run_bash`, `BashRuntime`, `VshCapability.open`, or new filesystem tool aliases.
- A compiled-out backend, missing binary, disallowed language and version mismatch have distinct actionable errors. No runtime download/installation occurs implicitly.

Python usage:

```python
# Requires this checkout's native build and matching bundled workers.
from vsh import BashConfig, Language, RunRequest, Runtime

runtime = Runtime.open("./workspace", bash=BashConfig())
preview = runtime.preview(
    "sed -i 's/debug=true/debug=false/' config/app.conf",
    language=Language.BASH,
    intent="Disable debug mode without changing other configuration.",
)
# Equivalent request-object form:
request = RunRequest("cat config/app.conf", language=Language.BASH)
read_preview = runtime.preview(request)
```

Rust usage:

```rust
// Requires this checkout's vsh `bash` feature and matching Bash worker.
let runtime = Runtime::open(
    RuntimeConfig::new("./workspace").with_bash(BashConfig::default()),
)?;
let preview = runtime.preview(
    RunRequest::new("sed -i 's/debug=true/debug=false/' config/app.conf")
        .with_language(Language::Bash)
        .with_intent("Disable debug mode without changing other configuration."),
)?;
```

Both snippets intentionally stop at preview. Approval/commit examples must use existing policy/hook flow, not suggest that exit code 0 grants commit authority.

### Result/API evolution

Native Rust replaces the Monty-specific receipt payload with a tagged `ExecutionOutput`: Monty value/text output or Bash exit code/binary stdout/stderr. Common transaction/decision/diff fields remain. Keep one canonical owned output object; do not maintain duplicated mutable byte/text copies.

This is a deliberate pre-1.0 Rust source change for callers accessing `Receipt.value` / `Receipt.stdout` directly. Record exact affected fields, update all owned consumers and document the migration in the selected release; do not pretend it is source-compatible by stuffing a Bash result into a fake `MontyObject`.

Python retains the observable Monty `Receipt.result` and `Receipt.stdout` behavior. For Bash, `result` is a typed `BashResult` carrying exit code and byte streams; text projections remain convenient receipt properties. Add language and stderr access without requiring users to decode an untyped dict. New `.pyi` overloads/types must match the compiled PyO3 surface. Failures expose similarly typed diagnostics via the existing exception family.

Extract common `ExecutionLimits`/stats only where truly backend-neutral. Preserve the current `ExecutionBudget` names/units and semantics; in particular, `max_os_calls` remains a documented count of guest FS requests, not actual kernel syscalls. Shell parser/work/intermediate-output limits belong in bounded Bash-specific settings. Specify precedence as effective limits bounded by both the host configuration and request; guest code cannot increase them.

## 12. Agent, MCP and packaging surfaces

### Pydantic AI and hooks

`VshCapability(..., bash=BashConfig(), hook_handler=judge.hook_handler)` enables Bash explicitly. Keep constructor-first usage and existing filesystem tools. Extend the existing `vsh_run` tool with language selection; its schema must advertise only enabled languages. Runtime validation is authoritative even if a caller bypasses the tool schema.

Keep sequential toolset behavior and async native-work offloading. Both deterministic handler and JEV examples must demonstrate preview/evidence → review → commit, plus a rejected/second-review case. Use deterministic test models for normal CI, not paid live calls.

MCP exposes the same language-aware existing run operation only when host-enabled. Codemode invokes that tool through its current tool surface; no second Bash interpreter inside the codemode Monty instance, no raw filesystem handle and no host process tool. CLI configuration must pass the same explicit backend config and render stdout/stderr/exit failures accurately.

### Distribution

Rust: `vsh` remains the main crate; internal `vsh-runtime` and other crate names stay unchanged. The new `bash` feature is default-off. Package the worker from `vsh-bash` with a clear native install/build command and version handshake.

Python: `vsh-python` remains the distribution, `import vsh` remains unchanged, and `vbash` remains the existing compatibility dependency package. Official wheels on validated Bash platforms are planned to include Bash support and the matching worker, while activation remains explicit. A Python extra cannot enable a Rust compile feature after installation; do not advertise a fictitious `[bash]` extra for that purpose. Quantify wheel-size/cold-import impact before accepting this packaging choice; if it is unacceptable, revisit distribution design before public API freeze.

Initial Bash platform targets are Linux and macOS, subject to conformance tests. Preserve the current Monty platform matrix. Do not advertise Bash on Windows until the selected profile's mode/commit semantics are satisfied; unsupported enablement must fail before execution rather than silently weaken the profile.

Update the existing PEP 517 wrapper to stage/validate both exact workers, including target triples, executable names/modes, sdist rebuilds and cleanup. Preserve the isolated docs-only Pages workflow: documentation builds must not compile either interpreter. No publish/push is authorized by this plan.

## 13. Performance design and measurement contract

Protect the common path first: no Bash worker startup, dynamic discovery, Tokio runtime, VFS mutex or Bashkit initialization on a Monty-only request. Use direct calls/closed dispatch, not a stack of boxed traits for every native filesystem operation.

Measure separately: snapshot, worker acquisition/handshake, parse/execute, FS RPC count/bytes, gateway work, canonical diff, binding/store, review serialization and commit. Exclude live LLM/network latency from core benchmarks; report judge invocation frequency, evidence bytes/tokens and costs separately when actually measured.

Known candidates, not promised optimizations:

- VFS append currently performs full read + extend + write. Repeated tiny appends can cause quadratic byte processing and excessive intermediate blobs. Count actual read/write/hash/materialization work, not just suffix bytes. Optimize only after evidence demonstrates the cost; do not discard intermediate effects or read dependencies for speed.
- Buffered pipelines need intermediate-byte accounting; `head` does not make an upstream full-buffer operation free.
- Metadata-heavy shell commands can cause many round trips. Start with correct narrow operations; consider bounded compound operations or transaction-local caching only with equivalent authorization/evidence and invalidation tests. Do not pre-authorize paths in the worker.
- Snapshot/VFS stays single-copy and parent-owned. Measure allocation and IPC copies before pursuing shared memory, zero-copy protocols, batching or a process pool redesign.
- Mode-only changes should avoid needless content materialization; preserve sparse overlay behavior on large snapshots.

Baseline and candidate must use the same fixtures, machine/toolchain/build profile and policies. Report p50/p95, cold/warm separation, peak parent/worker RSS, artifact bytes, IPC bytes/calls, operation counts and actual changed paths. Keep failures and limit-hit cases, not only fast successes.

Proposed regression gate: same-run paired Monty baseline/candidate median and p95 should not regress by more than 5% after noise controls, and no new snapshot-sized copy or per-call heap layer is acceptable. Treat this as a target to validate against measured noise, not a claimed current result. Record absolute deltas, repeated runs and confidence/spread; never manufacture a green gate by changing workloads. Bash latency budgets are set from Phase 0/benchmark evidence before public release, not invented here.

## 14. Release criteria and decision ledger

Implementation phases, explicit verification commands, negative test matrix and handoff requirements are in the [implementation plan](bashkit_implementation.md). A successful standalone Bashkit smoke test or clean dependency audit does not satisfy these release criteria.

| Decision | Reason/source | Revisit only if |
| --- | --- | --- |
| Parser and execution belong to Bashkit | User direction; shell semantics are not VSH's differentiator | Upstream cannot support the bounded contract after a concrete feasibility check |
| Parent owns VFS; worker uses RPC | Existing Monty worker architecture and policy ownership | Measured IPC cost is unacceptable and an equally strong design is demonstrated |
| Common gateway before Bash exposure | CallPolicy/budget code currently lives above VFS | Evidence shows a smaller extraction preserves exactly the same invariants |
| Subprocess-only Bash initially | Timeout/crash containment, no borrowed-VFS lifetime tricks | Benchmarks justify a separately named, explicitly weaker in-process mode |
| Mode primitive included | Upstream atomic `sed -i` requires chmod | Scope deliberately drops atomic editing, with user approval |
| Nonzero final exit is noncommittable | Observed partial-write behavior; safe existing failure boundary | A separately reviewed API explicitly supports partial-success proposals |
| No blanket deterministic-shell claim | Actual entropy/time behavior in upstream | A complete deterministic profile is proven, including nested execution and builtins |
| Feature-gated Rust / bundled Python workers | Existing distribution pattern and small Python install surface | Measured build/wheel cost invalidates this choice |
| Existing hook/JEV semantics remain | User-approved evidence-first review design | A separate review-design task changes the contract |
| Exact pin `=0.18.2` is the reviewed candidate | Current source/release and successful minimal RustSec scan | New advisory, required upstream fix or intentional reviewed upgrade |
| Disable upstream truncating stdout/stderr caps; fail on bounded intermediate/allocation/parent-output limits | P0 nested `sh` pipeline lost file bytes with exit 0 and no final truncation flag | Upstream provides proven lossless, sticky limit propagation |
| No output-rewriting `after_tool`/`after_exec` hooks | P0 identity hook replaced non-UTF-8 bytes | Hooks gain a verified lossless byte contract |
| Deny-only command-resolution observer plus before-dispatch profile guard | Nested dispatch is observable; unresolved noninteractive aliases otherwise evade the builtin hook | A supported equivalent observer exists |
| Parent deadline is authoritative | A 20 ms cooperative loop timeout took about 120 ms in P0 | Never: cooperative limits complement process supervision |

Local P0 evidence and reproduction commands are recorded in [the evidence report](bashkit_p0_evidence.md). Cross-platform mode conformance, the combined graph audit and production gateway/worker checks remain separate gates; upstream-only tests do not close them.

Before coding, Phase 0 must close worker memory enforcement, unsupported/truncation propagation, necessary builtin FS calls and platform-mode support. Those are bounded feasibility gates with selected defaults, not an excuse to grow a generic operating system. Changes to the chosen contract must update both plans before implementation proceeds.
