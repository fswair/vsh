# Custom tools on the active VSH filesystem

Status: recorded proposal; implementation deferred until the current API stabilizes.
Recorded: 2026-09-06.
Scope of this change: this planning document only.
Feature version terminology: **v0** means the first custom-tool iteration, not a VSH
package version. No release number or delivery date is committed here.

This file preserves the product discussion, system design, and a phased implementation
plan so the work can resume without reconstructing the conversation. It is a durable
repository plan, not a published API reference. All new names and signatures below are
illustrative and must be reviewed before implementation.

## 1. Product decision and timing

The proposed feature is worth adding when a user-defined domain tool must execute
inside the same virtual transaction as existing VSH operations. Examples include
updating a service configuration, generating migration files, and coordinating a
component rename. The application owns the domain logic; VSH owns the filesystem
semantics, evidence, policy, review, and commit.

The immediate priority is stabilizing the current Rust/Python API, commit hooks,
Pydantic AI capability, and judge integration. Recording this plan is not authorization
to implement the feature now. Stabilization must not acquire a speculative registry,
placeholder exports, new dependencies, or a partially implemented custom-tool API.

The useful product increment is:

> Applications register typed domain functions whose filesystem work joins the active
> VSH snapshot. Reads observe earlier virtual writes; resulting changes and dependencies
> enter the existing review and commit pipeline.

It must preserve VSH's priorities: low simulation latency, bounded memory, explicit
authority, one Rust semantic core, and matching Rust/Python behavior.

### When a registry earns its cost

Today an application can wrap `VshCapability.vsh_patch()` or `.vsh_run()` in its own
Pydantic AI tool. That composition is sufficient when each domain call is an independent
transaction. It does not make that Python wrapper callable from the middle of an
already-running Monty program or share that program's active overlay.

Proceed when representative workflows require this shared execution:

```text
vsh_write('/workspace/config/service.toml', draft)
set_service_timeout(30)                 # custom tool sees draft
report = inspect_service_config()      # custom reader sees the timeout edit
report
```

All three steps must contribute to one simulation and one final transaction. Opening
a second runtime or calling `Runtime.run()` from the custom handler cannot provide that
property and must not be an implementation shortcut.

### Stabilization entry gate

Before starting the feature, establish evidence for:

- The intended released shape of `Runtime`, `HookedRuntime`, `VshCapability`,
  `CommitJudge`, and their sync/async behavior is settled.
- Installation, worker discovery, exact package identity, and documented examples work
  from clean environments; optional-provider compatibility is explicit.
- Existing policy, review, cancellation, stale-state, and recovery contracts pass their
  maintained checks. Known blockers have either been fixed or explicitly scoped.
- Documentation distinguishes shipped behavior from development-only behavior.
- Fresh baseline measurements cover native Rust, Python/PyO3, Monty worker execution,
  and compound transactions, with machine/build configuration recorded.
- At least two realistic domain workflows demonstrate why independent tool wrappers
  are insufficient.

Revisit this gate with the actual implementation and dependency versions at that time.
The current worktree and previous local test results are context, not future release
certification.

## 2. Discussion preserved: decisions and alternatives

| Idea discussed | Disposition | Reason |
|---|---|---|
| Explicit application-defined tool name, description, schema, handler | Include in v0 | Domain logic and authority are host-owned |
| Tools usable inside Monty alongside built-ins | Include in v0 | Shared overlay and one transaction are the feature's value |
| Reader versus possible filesystem writer | Include as an enforced ceiling | A reader declaration must prohibit virtual writes |
| Infer mutation from selected argument fields | Defer | Arguments can predict intent, but observed operations determine effects |
| Handler returns a filesystem mutation proposal | Preserve as a later alternative | Potentially useful for planners and batching; not needed alongside a context API in v0 |
| Give handler access to the active virtual filesystem | Preferred v0 direction | Reuses native observation, diff, policy, and commit semantics |
| Handler supplies final `DiffEntry` / `CanonicalChange` | Do not accept as authoritative input | Canonical evidence and dependencies must be derived by VSH |
| Transparently virtualize an arbitrary existing host tool | Out of v0 scope | Tools must explicitly use the supplied filesystem interface |
| External reads or non-filesystem mutations | Separate future investigation | Freshness, approval, commit, and recovery need different contracts |
| General plugin discovery / package marketplace | Out of scope | Explicit application configuration is enough |

The earlier `ToolOutcome(operations=[FsPatch(...)])` sketch was an alternative design,
not a final API decision. The preferred first iteration uses a scoped filesystem context
and ordinary typed return values. Do not ship both authoring styles without a measured
need. Do not resurrect the removed Python simulator, extension registry, or their
compatibility aliases to implement this feature.

## 3. Current system and actual extension seam

The following paths were inspected when this plan was recorded. They are implementation
evidence, not a promise that paths or private function names will stay unchanged.

| Area | Current location | Relevant behavior |
|---|---|---|
| Runtime orchestration | [runtime.rs](../crates/vbash/src/runtime.rs) | Owns snapshot, execution, transaction binding, policy, and commit resolution |
| Monty injected functions | [tools.rs](../crates/vsh-monty/src/tools.rs) | `inputs()`, fixed names/specs, `is_tool()`, and `dispatch()` |
| In-process Monty adapter | [lib.rs](../crates/vsh-monty/src/lib.rs) | Handles typed OS calls and function suspensions; owns authorization and budgets |
| Worker parent adapter | [worker.rs](../crates/vsh-monty/src/worker.rs) | `resume_tool_call()` validates and dispatches suspended function calls |
| Virtual filesystem | [lib.rs](../crates/vsh-vfs/src/lib.rs) | Records observations/effects, maintains overlay and read/write sets, produces canonical diff |
| Canonical value types | [lib.rs](../crates/vsh-types/src/lib.rs) | `DiffEntry`, `DiffKind`, identity digests, transaction lifecycle |
| Hook evidence | [hook.rs](../crates/vbash/src/hook.rs) | Immutable `RequestEvent`, `EffectEvent` evidence, `CommitHook` |
| Durable evidence | [artifact.rs](../crates/vbash/src/artifact.rs) | Encodes/validates transaction artifacts and effect provenance |
| Captured review bytes | [review.rs](../crates/vbash/src/review.rs) | Collects bounded immutable content from canonical changes and read effects |
| Python native boundary | [lib.rs](../crates/vsh-python/src/lib.rs) | PyO3 runtime bindings, error conversion, immutable evidence projections |
| Python hook coordination | [hooks.py](../src/vsh/hooks.py) | Prepares, invokes, and resolves sync/async review handlers |
| Pydantic AI adapter | [pydantic_ai.py](../src/vsh/pydantic_ai.py) | Eleven agent tools and committed-result/feedback conversion |
| Judge | [_judge.py](../src/vsh/_judge.py) | Renders bounded evidence and validates structured approval references |

The directory named `crates/vbash` currently contains the `vsh-runtime` implementation;
package names must be checked against Cargo manifests when work resumes.

Monty already suspends external function calls. Both execution paths currently reject
object-method calls and names outside the fixed built-in list. The parent owns the
active `VirtualFs`; the child worker receives function metadata and values, not a host
filesystem mount. This is a useful seam for explicit registration.

Registration still requires real work: callable discovery alone does not establish
argument validation, authorization, resource accounting, callback lifetime, provenance,
or Python/Rust parity. The two execution paths must share the same new dispatch rules.

### Reuse existing semantics correctly

- Rust `DiffEntry` is one canonical before/after path change. Python
  `CanonicalChange` is its immutable projection.
- `Effect` / `EffectEvent` record observed operations, including reads and writes.
- `VirtualFs` owns actual overlay state, dependencies, and canonicalization.
- Authorization and resource charging are currently also performed in the Monty
  adapter. A bare `&mut VirtualFs` is not a complete policy/budget boundary.

Consequently, the handler must receive a guarded view built from these components.
Reusing `VirtualFs` does not mean exposing its unrestricted mutation API or accepting
fabricated effect records from a callback.

## 4. First iteration: v0 scope

### Included

1. Explicit registration supplied by trusted application code before runtime use.
2. Tool name, description, validated input/output contract, semantic version identity,
   filesystem access ceiling, and synchronous handler.
3. Runtime-owned immutable registry shared by Python and native Rust surfaces.
4. Scoped read/write operations on the current transaction, including read-your-writes.
5. Runtime enforcement of reader declarations, native path policy, and budgets.
6. Existing native effect recording, read/write dependency tracking, canonical diff,
   policy evaluation, hook/judge review, stale checks, and exact commit.
7. Bounded tool-call provenance visible to trusted review and audit surfaces.
8. Consistent behavior under in-process and supervised-worker execution.
9. Python callback support through PyO3 without making Pydantic AI a core dependency.
10. Pydantic AI mounting that reaches the same native implementation when enabled.

### Excluded

- Hot registry replacement during a running or pending transaction.
- Agent-supplied handler code, dynamic imports, arbitrary module loading, or `eval`.
- Automatically intercepting `open()`, `pathlib`, subprocesses, SDK calls, or network
  I/O inside an existing Python/Rust host handler.
- Raw host paths, commit/approval methods, store handles, or unrestricted `VirtualFs`
  ownership inside a tool context.
- Async custom handlers, custom-to-custom callback recursion, retained contexts, and
  handler-spawned threads operating on the context.
- A user-authored canonical diff or a second filesystem simulator in Python.
- Call-level savepoints, implicit retries, result caching, durable callback replay, or
  streaming custom-tool output.
- Non-filesystem actions such as HTTP writes, database changes, email, or deployments.

These exclusions describe the first feature boundary. The future-stage sections preserve
the ideas that may become worthwhile once real use demonstrates their value.

## 5. Trust, ownership, and enforcement

Three actors have different authority:

| Actor | Controls | Must not control |
|---|---|---|
| Application author | Tool implementation, schema, registry, permission ceiling, review rules | Whether VSH silently accepts inconsistent evidence |
| Agent / Monty guest | Calls to registered names, validated arguments, bounded guest computation | Handler registration, permission increases, registry identity, approval authority |
| Rust runtime | Allowed virtual operations, budgets, observations, canonical artifacts, commit | Application-specific meaning of a config or generated file |

Custom callbacks execute as **trusted host application code**. Giving a Python callable
a filesystem view does not sandbox the callable itself: it can still import libraries,
open a real file, or make a network call using its ambient host authority. VSH cannot
observe or roll back those actions. The supported contract requires the handler to use
the supplied view for all workspace I/O. Untrusted handlers require separate process
isolation and a different security design.

Similarly, an arbitrary Rust handler is native trusted code. Memory allocation,
unrelated I/O, panics, and non-termination require honest application/deployment limits.
Do not advertise host callbacks as constrained by Monty's heap or CPU quotas.

### Guarded filesystem view

The proposed view is tied to one active callback and one transaction. It must:

- Normalize paths into the same synthetic namespace as built-ins.
- Intersect native call policy with the registered tool's access ceiling.
- Charge actual reads, writes, directory visits, and outputs against the enclosing
  execution budget; a custom tool does not receive a fresh budget per sub-operation.
- Use the same active overlay and native observation methods as built-ins.
- Record failed denied accesses consistently, even if handler code catches an error.
- Expose neither host handles nor blob-store/admin APIs.
- Reject use after callback completion, transaction cancellation, or execution failure.
- Reject cross-thread and recursive runtime entry in v0.

The runtime, not the handler, supplies dependency records and before-state identities.
Metadata, negative existence checks, directory listings, and content reads must all keep
their current stale-detection behavior. Merely hashing arguments is insufficient.

### Reader/mutator semantics

A declaration such as `read_only` is an enforced maximum permission. Every attempted
write must fail before overlay mutation, including no-op writes, append, rename,
metadata changes, and writes hidden inside a higher-level helper.

A declaration such as `filesystem_write` permits asking for writes; native policy may
still deny them. It does not claim that a particular invocation actually mutated state.
Bounded pure computation is allowed with either ceiling, but ambient external reads
are outside the supported filesystem evidence contract.

Keep these distinctions in evidence:

| Invocation | Observed behavior | Final canonical diff |
|---|---|---|
| Reader parses an existing file | Reads and dependencies | Empty |
| Write-capable tool takes an analysis-only branch | Reads / computation | Empty |
| Tool changes a file | Mutation effects | Non-empty |
| Tool writes and restores original bytes | Mutation effects still occurred | May be empty |
| Reader attempts a forbidden write | Contract violation | No authorized change |

An empty diff does not prove the execution was read-only. Classification and review
must preserve ordered effects and violations, not merely inspect final changed paths.

The original suggestion of selecting "mutator parameters" can later annotate expected
behavior for UI or preflight checks. It must never suppress actual-effect tracking or
grant extra permission. `dry_run=True` is not evidence that no write was attempted.

## 6. Proposed authoring experience

The following is design notation, **not an existing or executable API**. Names such as
`ToolSpec`, `ToolContext`, and `tools=` are placeholders. Keep the final surface small;
do not publish decorators, builders, a registry service, and proposal objects for the
same task by default.

```python
# PROPOSED ONLY — these custom-tool types and options do not exist today.
def set_service_timeout(ctx: ToolContext, seconds: int) -> dict[str, int]:
    if not 1 <= seconds <= 120:
        raise ValueError("seconds must be between 1 and 120")
    config = parse_service_config(ctx.fs.read_text("/workspace/config/service.toml"))
    config.timeout_seconds = seconds
    ctx.fs.write_text("/workspace/config/service.toml", render_service_config(config))
    return {"timeout_seconds": seconds}

tool = ToolSpec(
    name="set_service_timeout",
    description="Update the service timeout while preserving other configuration.",
    version="1",
    parameters=timeout_parameters,
    result=timeout_result,
    access="filesystem_write",
    handler=set_service_timeout,
)
runtime = Runtime.open(workspace, tools=[tool])
capability = VshCapability(workspace, tools=[tool], hook_handler=review_handler)
```

The parser and renderer belong to application code. They illustrate why a domain tool
may need host libraries that Monty does not provide. The callback must read and write
through `ctx.fs` even when its parsing library runs on the host.

The two runtime/capability constructions illustrate alternative entry points, not two
runtimes to use for one transaction. `Runtime.open()` remains the runtime constructor;
`VshCapability(...)` remains constructor-first. Registration must not recreate the
removed `VshCapability.open()` surface.

For Rust, prefer one normalized descriptor plus a bounded callback/trait operating on
a scoped context with borrowed lifetime. A native handler returns supported typed values;
it does not import Python, construct Python toolsets, or produce a canonical diff.
The exact trait signatures and `Send`/`Sync` requirements are a design-spike outcome.

### Arguments and schema

- Bind positional/keyword arguments consistently in both Monty execution paths.
- Validate required fields, extra fields, numeric bounds, enum values, nesting, and
  serialized input size before handler invocation.
- Validate and bound return values before resuming the guest.
- Choose and document a supported schema/value subset. Do not promise arbitrary JSON
  Schema or arbitrary Python object serialization without implementing them.
- Core contracts must not require importing Pydantic AI. Optional Python schema adapters
  may derive descriptions from typing/Pydantic metadata and normalize into the Rust
  contract; unknown schema constructs should fail registration explicitly.
- Preserve parameter descriptions and return metadata for Pydantic AI and Monty docs.
- Never format unescaped arguments into generated Python/Monty source. Use typed value
  injection or the existing validated argument path.

### Names and registration

Validate the registry once when constructing the runtime: unique identifiers, bounded
schema/description sizes, valid callable binding, and explicit versions. Reserve existing
`vsh_*` functions and runtime/internal names. Reject collisions and unknown callable or
object-method dispatch. Freeze the registry before its first transaction.

Registry lookup must remain separate from the built-in fast path. Merely importing VSH
or constructing a runtime without custom tools must not initialize Python callbacks,
build custom schemas, or allocate a registry per transaction.

## 7. Execution path and system responsibilities

```text
trusted application: descriptors + handlers
             │ validate, normalize, freeze, bind identity
             ▼
runtime-owned registry ─── descriptors only ───► Monty worker
             ▲                                     │ named function suspension
             │                                     ▼
             └──────── parent dispatch ◄──── bounded typed name/arguments
                              │
                    argument validation
                              │
                    scoped trusted handler
                              │ guarded fs operations
                              ▼
             authorization + shared budget + active VirtualFs
                              │ native observations / effects
                              ▼
              canonical diff + dependencies + tool provenance
                              │
                       policy → hook/judge
                              │
                 exact artifact → revalidate → commit
```

### Registration and worker protocol

Inspect the existing worker input/function-value protocol before inventing new wire
messages. It already transports built-in function objects and names. Use that mechanism
for descriptor injection if it can enforce the new contract; extend the VSH wrapper only
where validation or metadata actually needs it. No upstream Monty fork is presumed.

The worker never receives a Python callable, Rust trait object, pointer, host path, or
credential. Both worker and in-process execution resolve the same normalized descriptor
and invoke the same parent-side guarded dispatch. Update protocol/version handshakes if
required by a wire change; reject incompatible workers explicitly.

### One transaction

Successful custom calls update the current virtual overlay immediately. Later built-ins
and custom calls observe those updates. Successful function return is not a host commit.
Policy and commit hooks run after the enclosing simulation freezes its final artifact.
Do not introduce per-tool approvals or hidden commits inside a Monty transaction.

Commit promotes the stored artifact and must not rerun handlers. Re-execution may have
different library versions, external ambient state, or time-dependent behavior; it cannot
reuse an approval for a previous artifact.

### Failure behavior

For v0, a custom handler failure after dispatch begins invalidates the enclosing
simulation for commit. This includes a Python exception, malformed return, Rust callback
panic handled at the boundary, context misuse, and permission/budget violations. Even
if guest code catches a surfaced error, partial callback effects must not become
committable. Use a fatal/tainted execution outcome instead of adding public lifecycle
states or implicitly approving the earlier portion of the transaction.

Document registration/argument errors separately from pending review. Failed simulation
is not the same thing as a successful simulation whose changes await approval. Use the
existing execution error family where it fits and add a typed category only if needed.
Avoid raw callback exception text in model-facing errors: it may contain paths, content,
or credentials. Preserve useful bounded diagnostics for trusted application logging.

No automatic callback retries in v0. A repeated call is additional execution and must
consume resources and record evidence. Applications may submit a new transaction after
correcting the problem.

### Python callback lifetime and concurrency

The normal native execution path releases the GIL. A Python callback requires GIL
reacquisition and value conversion only for that call. Rust callbacks remain native.
Do not serialize independent Rust transactions behind a global Python registry lock.

The implementation spike must establish an owned transaction handle/lease design that
does not expose raw borrowed pointers to Python. Only the currently active handler may
use its view, on the authorized thread. Expire the lease on completion or failure; a
retained Python reference must be harmless. Do not hold mutable VFS borrows, registry
locks, or commit coordination locks across reentrant Python code.

Synchronous v0 callbacks are simpler, but **a deadline cannot forcibly interrupt an
arbitrary in-process Python or Rust callback**. Check deadlines at dispatch, filesystem
operations, and return, and refuse late commit. Monty worker termination does not kill a
callback running in the parent process. Cancellation must revoke access and block commit;
bounded return latency for a non-cooperative callback requires process isolation.

The GIL/lease/reentrancy/cancellation spike is a go/no-go gate. If safe semantics require
new process isolation or a full async continuation engine, revisit v0's cost and scope
before implementation proceeds. Do not claim safety from a `to_thread()` timeout alone.

## 8. Evidence, review, persistence, and identity

The current native effect records must continue describing actual filesystem work.
Add enough custom-call provenance to explain which registered tool caused it:

- Transaction-local call ID, tool name, registered semantic version, and contract digest.
- Validated argument and result identities using a defined canonical encoding.
- Ordered association between a tool call and its filesystem effects/dependencies.
- Declared access ceiling, actual observed activity, and bounded status/failure metadata.

Do not replace filesystem effects with a tool author's claim that "this was a read."
Do not accept callback-supplied policy scores, completeness flags, or before-state
hashes as authority.

### Identity and pending transactions

Bind a deterministic registry/configuration digest into the transaction's runtime
identity and durable artifact. Normalize descriptor ordering and schema representation
before hashing. Include behavior-affecting tool configuration and explicit semantic
version identity in the binding design.

A schema digest does not prove the handler code is unchanged. An application-declared
version is an audit commitment, not code attestation. Opaque callable source hashing is
not a reliable general substitute. If automatic reproducibility is required, a future
deployment artifact identity must cover handler code, libraries, and configuration.

Define a conservative reopen rule: a pending custom-tool artifact may be reviewed and
committed only under a compatible verified registry identity. Registry/config mismatch
must not silently inherit old approvals. Commit still uses frozen bytes; it must not
need the callback to regenerate a proposal. Recovery of a previously started native
commit must remain possible from its durable journal without executing extension code.

### Hooks and judge

Existing `RequestEvent` semantics remain transaction-based. `HookScope.REVIEW_REQUIRED`
and `HookScope.ALL_REQUESTS` keep their current meaning; registering a custom tool
does not silently widen review scope. A model judge may approve pending work, never
override a native hard denial.

Custom-call metadata must survive persistence and be included in the supported hook
evidence version before it is advertised to reviewers. Update the judge renderer and
evidence-reference validation to cover material new records. Do not silently drop them
or report `evidence_complete=True` when a required tool record was truncated. Unknown
required evidence versions fail closed for new custom-tool approvals.

Arguments and results can contain secrets independent of file content. The existing
`content_filter` authorizes file bytes; it is not permission to disclose arbitrary custom
arguments or outputs. Default to bounded metadata/digests, define explicit disclosure
rules, and keep all strings untrusted. When semantic review requires withheld values,
return review instead of asking the judge to infer values from hashes.

Digests are identities, not anonymization: low-entropy values can be guessed and hashed,
and stable digests can link sensitive calls. Apply access controls to persisted identities
as well as plaintext; omit sensitive value identities from model-visible evidence unless
explicitly authorized. If cross-call identity is necessary, assess keyed identities and
their key lifecycle during the evidence design instead of assuming a plain hash is safe.

Within Monty, a custom return value can be used by later guest code in the same
transaction. At the Pydantic AI boundary, the existing committed-result gate must still
hold: no shortcut may release pending/rejected guest values. A disclosure review cannot
undo data already read by a trusted host handler; document this distinction.

## 9. Pydantic AI mounting and SDK parity

The registry belongs to VSH's native runtime. Pydantic AI is an optional consumer of
the same normalized descriptors, not the owner of dispatch or mutation classification.

A registered tool should be available to `vsh_run` inside the active Monty transaction.
If exposed as a separate Pydantic AI tool, that tool starts one VSH transaction through
the native invocation path and returns the usual `VshToolResult`. It must never call
the host handler directly and bypass simulation/review.

Start with a small, deliberate mounting choice. Verify whether applications need a
domain-only toolset, built-ins plus domain tools, or just `vsh_run` discovery; finalize
that choice during the design spike. Do not proliferate public mounting flags without
real examples. Pydantic AI names/descriptions/schema and Monty docs must agree.

Native Rust and Python must support the same registration rules, filesystem behavior,
error categories, evidence, and commit semantics. Adapter value conversion may differ,
but Python must not build its own `CanonicalDiff` or maintain a shadow overlay.
The native Rust dependency graph must not acquire Pydantic AI or Python dependencies.

## 10. Performance plan

There are no custom-tool measurements yet. The following are measurement requirements,
not claims of achieved performance.

| Compare | What it establishes |
|---|---|
| Current built-ins versus feature build with no registry | Baseline cost of an unused feature |
| Built-ins with a registry present but unused | Lookup/configuration overhead |
| Equivalent built-in operations versus one Rust domain callback | Native dispatch and validation overhead |
| Equivalent native callback versus Python callback | PyO3/GIL and conversion cost |
| Many small virtual operations in one callback | Cost per scoped filesystem call |
| Independent tool transactions versus one compound Monty transaction | Snapshot/IPC savings against registration overhead |
| Large permitted args/results, content, and directories | Peak memory, copying, and budget enforcement |
| Concurrent independent workspaces and overlapping same-workspace work | Throughput, tail latency, contention, stale behavior |

Record cold/warm p50/p95/p99, throughput, parent/worker peak memory, allocations or copy
proxies where available, suspension count, and schema-validation/callback time. Separate
simulation from durable review capture and commit. Keep source, workspace, compiler
profile, runtime configuration, worker version, and repetitions comparable.

Design constraints:

- No extra worker round trip for each `ctx.fs` call when the handler is already executing
  in the parent with the active filesystem.
- Validate/cache descriptors at registration; avoid per-call schema compilation.
- Keep built-in dispatch direct and native when no custom registry is configured.
- No global lock across user code, fresh interpreter per call, hidden network schema
  resolution, or unbounded intermediate operation list.
- Count custom call arguments/results, inner filesystem operations, and provenance
  storage separately; final output caps alone do not bound intermediates.
- A Python tool can be slower than a Rust tool. Measure when domain-level batching
  offsets that overhead rather than claiming universal speedups.

Set numerical regression thresholds from repeated stabilized baselines before merging
implementation. If the no-registry path regresses beyond measurement noise, investigate
the architecture before accepting an extension tax. If custom tools do not beat or
simplify realistic composition, defer the feature further.

## 11. Validation matrix for implementation

These are future acceptance requirements, not tests to add while only recording this
plan. Exercise public behavior through Rust and Python and both Monty execution paths.

| Case | Required outcome |
|---|---|
| Built-in write followed by custom read | Custom reader sees the same overlay |
| Custom write followed by built-in read | Guest observes proposed bytes before commit |
| Two dependent custom calls | Ordered effects, one transaction |
| Preview containing custom mutation | Host bytes unchanged |
| Approved custom transaction | Exactly stored bytes applied once; handler not rerun |
| Host dependency changes while review waits | Stale rejection before reviewed mutation |
| Reader attempts write/rename/metadata change | Permission violation; no committable artifact |
| Write-capable callback only reads | Read dependencies preserved; no invented mutation |
| Write followed by restoration | Empty final diff possible, mutation effects retained |
| Invalid/missing/oversized arguments | Handler never runs |
| Invalid/oversized return after writes | Whole simulation becomes non-committable |
| Callback writes then raises; guest catches it | Partial effects never commit |
| Handler catches a denied operation internally | Violation still recorded and enforced |
| Traversal, protected path, symlink, recursive path operation | Same controls as built-ins |
| Context retained, reused, cross-thread, or used after cancellation | No further filesystem authority |
| Nested runtime or callback reentry | Explicit rejection, no deadlock |
| Non-cooperative callback exceeds deadline | No late commit; documented lack of host preemption |
| Duplicate name, reserved name, unsupported schema | Registration fails before execution |
| Registry/version change after persisted preview | Incompatible approval/commit rejected |
| Worker pooled across differing registries | No stale names/handlers or tenant authority |
| Truncated/unknown custom-call evidence | Cannot be approved as complete |
| Prompt injection in schema/args/results/content | Data does not become judge authority |
| Pending Pydantic AI custom tool | Feedback returned, guest value withheld |
| Runtime without custom tools | Existing API and measured hot path preserved |

Add property/fuzz coverage where it protects canonical argument conversion, registry
identity, bounded parsing, and durable evidence decoding. Keep model-quality evaluation
separate from deterministic correctness tests. Run optional live model trials only after
the native safety contract and offline Pydantic AI wiring work.

## 12. Phased implementation plan

All implementation phases below are **not started**. Phase boundaries are feature
milestones, not package releases. Re-open this plan and refresh evidence before work.

### Phase A — stabilize existing API (priority now)

Objective: complete the stabilization gate in section 1 using the current product scope.
Relevant areas: existing SDK/hook/judge code, package validation, documentation, CI,
and benchmarks. Do not add registry scaffolding as part of this phase.

Exit evidence: current surfaces and installation paths are verified; unresolved API
decisions and performance baselines are recorded. Starting custom-tool development is
a separate product decision after this gate.

### Phase B — bounded design spike

Objective: prove the difficult seam before selecting final public names.

1. Build two application-level domain scenarios using current composition and record
   the limitations that require same-overlay dispatch.
2. Verify dynamic descriptors can use the current Monty function-value wire path.
3. Prototype a guarded context over the existing authorization/budget/VFS layers.
4. Prove PyO3 lifetime, GIL, reentrancy, failure-taint, and cancellation rules.
5. Choose the supported schema/value subset and normalize it across Rust/Python.
6. Specify evidence/disclosure identity and registry reopen behavior.
7. Measure no-registry overhead plus a native and Python callback.

Expected areas: `crates/vsh-monty`, `crates/vsh-vfs`, `crates/vsh-python`, and temporary
fixtures. Keep prototypes private or isolated; they are not releasable exports.
Exit: feasibility, go/no-go risks, final v0 boundary, and numeric performance gates.

### Phase C — native guarded dispatch

Objective: implement registry, schema binding, invocation scope, and error semantics
once in Rust, behind an explicit runtime configuration.

1. Introduce the minimal normalized descriptor/handler contract in a dependency layer
   that avoids a `vsh-runtime` ↔ `vsh-monty` cycle; prefer a focused module before a crate.
2. Share authorization/budgeted filesystem primitives between built-ins and callbacks.
3. Freeze registry identity at runtime construction; preserve no-registry dispatch.
4. Integrate in-process and worker suspension paths and name collision checks.
5. Implement access ceiling, scoped lifetime, shared accounting, and fatal callback
   failure handling.

Expected areas: `crates/vsh-monty`, runtime configuration, native exports, targeted tests.
Exit: native composition, policy, budgets, failures, and parity cases pass; built-ins
retain their semantics and measured performance.

### Phase D — bound evidence and durable review

Objective: make custom work reviewable and recoverable before exposing automatic commit.

1. Bind registry identity and call provenance to the exact transaction.
2. Extend durable artifact codecs, version checks, completeness, and size bounds.
3. Expose immutable custom-call evidence through Rust and Python `RequestEvent`.
4. Preserve hook scopes, hard-deny behavior, and single-use approvals.
5. Specify and test pending reopen, configuration mismatch, stale resolution, and
   recovery without callback replay.
6. Extend judge evidence rendering/reference validation with explicit argument/result
   disclosure; withheld required evidence must remain reviewable only by appropriate hosts.

Expected areas: runtime, hooks, artifacts, review, store/type definitions as needed,
PyO3 evidence projections, `_judge.py`. Exit: full durable lifecycle passes the security
matrix; no custom transaction can bypass evidence binding.

### Phase E — Python and Pydantic AI integration

Objective: expose the tested native contract with a small Python authoring surface.

1. Add typed callable conversion and expiring context bindings without exposing raw VFS.
2. Add optional schema convenience only where it simplifies actual application code.
3. Wire construction into `Runtime.open` and constructor-first `VshCapability`.
4. Mount custom descriptions/schema into Pydantic AI through the same native dispatcher.
5. Preserve result gating, error sanitization, feedback, and sequential tool behavior.
6. Run two complete domain examples with deterministic hooks and offline judge review.

Expected areas: `crates/vsh-python`, Python exports/stubs, capability adapter, examples,
tests. Exit: Rust/Python parity, typing, sync callback rules, and agent-facing behavior.

### Phase F — v0 acceptance and release decision

Objective: decide whether the proven feature warrants a public release.

1. Compare implementation against this plan; resolve differences explicitly.
2. Run full existing quality gates plus the new acceptance cases on supported platforms.
3. Measure the matrix in section 10 and publish reproducible results with limitations.
4. Write separate Rust/Python references and guided domain-tool examples using final APIs.
5. Explain trusted-handler authority, non-preemptible callbacks, explicit filesystem use,
   and the boundary of VSH's commit guarantees.
6. Validate registry-free package behavior, worker versioning, artifacts, and optional
   dependencies. Refresh maintained/security-reviewed dependency pins only if new ones
   were actually necessary.

Use the repository's then-current CI commands. Starting reference commands are:

```bash
cargo fmt --all -- --check
cargo test --workspace --all-features --all-targets --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
uv run ruff check
uv run ruff format --check
uv run basedpyright
uv run pytest --cov=src/vsh --cov-branch --cov-report=term-missing --cov-fail-under=100
```

Also run the maintained `ty`, Rust coverage, distribution/consumer smoke, dependency,
and documentation checks from [CI](../.github/workflows/ci.yml) and
[development guidance](../docs/development.md). Passing local tests is not authorization
to tag, publish, or push. Do not add tests for removed API names merely to retain them.

## 13. Later stages, preserved without premature APIs

### Stage 1 — authoring ergonomics and domain libraries

After v0 usage reveals repetition, consider schema derivation, reusable domain toolsets,
more precise per-tool path ceilings, or Monty-defined compositions of built-ins. Keep
one normalized native contract and measure model schema/context size. Add a decorator
or builder only if it reduces real boilerplate while preserving descriptions and types.

### Stage 2 — proposal-returning or batched handlers

Revisit the user's original argument-to-filesystem-proposal idea when a workflow needs
pure planning, serialization, or fewer Python-to-Rust calls.

A possible handler would read through a guarded view and return bounded typed operation
requests plus a value. These would be executed through the same authorization/budgeted
VFS gateway, which produces authoritative `EffectEvent` and `DiffEntry` records.
The proposed operations are requests, not trusted canonical changes.

Before adding an operation type, inspect existing native typed calls for reuse. If a
new public command representation is unavoidable, distinguish it from canonical effect
outputs and keep the supported operation set explicit. Avoid making users construct
blob identifiers or before-state hashes.

Questions to resolve: ordered reads-after-proposed-writes, returning actual operation
results, preconditions, size/copy costs, all-or-nothing failure, and whether callers now
need two mental models. Prefer an optimization beneath the context surface if that
provides the same benefit. Do not build this merely to mirror `ctx.fs.write(...)`.

### Stage 3 — async callbacks and controlled external reads

Async callbacks require a proper suspension/cancellation and transaction-lease model,
not a blanket thread wrapper. Never hold VFS borrows or commit locks across arbitrary
awaits. Establish isolation and authority expiry before claiming bounded cancellation.

External reads introduce freshness and disclosure problems: service identity, version
tokens/ETags, immutable captured values, revalidation, missing-dependency behavior,
timeouts, credentials, and egress authorization. Arbitrary external state cannot be
promised the same snapshot isolation as a local filesystem. Domain contracts must
state the guarantee they can actually provide.

### Stage 4 — non-filesystem effects (separate proposal)

Database changes, remote writes, messages, and deployments need an explicit effect
provider contract and a separate product decision. A future design might require:

- Side-effect-free preparation with evidence and deterministic operation identity.
- Host-owned authorization and disclosure rules for the remote system.
- Idempotency, version checks, bounded retries, and persistent delivery records.
- Commit/revalidation ordering, crash recovery, partial failure, and compensation rules.
- An explicit statement of atomicity limitations across local and remote systems.

An outbox can provide durable post-filesystem delivery, not one atomic transaction with
an arbitrary remote API. Compensation is not rollback, and an ordinary hook callback
cannot manufacture distributed atomicity. This stage may remain out of VSH entirely.

### Stage 5 — parameter annotations and preflight UX

If users need it, allow explicit descriptions of which arguments request a write,
influence a path, or require extra review. Treat these as explanatory or restrictive
preflight contracts. Runtime observations remain authoritative; annotations cannot
downgrade a real mutation, erase transient effects, or turn `dry_run` into a permission
bypass. Derive model/UI descriptions from one contract rather than maintaining two.

## 14. Decision ledger and revisit questions

| Decision | Basis | Revisit when |
|---|---|---|
| Stabilize current APIs first | User's requested sequencing | Existing stabilization gate is met |
| Same active overlay is the feature's core | Discussion and current separate-call semantics | Real domain workflows do not need shared composition |
| Guarded filesystem context is preferred v0 | Existing native operations already derive correct evidence | A measured planner/batch workflow favors proposal returns |
| Native effects and diff are authoritative | Current commit/evidence design | No planned relaxation |
| Reader is enforced; write classification is observed | Mutation may depend on arguments or cancel out in final diff | Only terminology/ergonomics may change |
| Explicit trusted host handlers only | Existing parent-side suspension seam | Separate isolation proposal is justified |
| Sync callbacks first, no fabricated preemption guarantee | PyO3/Rust host callback mechanics | Async or isolated implementation proves better lifecycle semantics |
| Immutable registry and version binding | Pending evidence must retain its meaning | Safe versioned replacement/recovery design is demonstrated |
| External effects are outside v0 | VSH commits filesystem artifacts | Separate effect-provider product proposal is approved |

Before implementation, settle these questions with code and measurements:

1. Which minimum schema/value subset covers both real domain examples in Rust/Python?
2. Where can the guarded context and registry live without a crate dependency cycle?
3. Can Python callbacks safely access the active transaction without raw pointers or
   lock/reentrancy hazards, and what can cancellation actually guarantee?
4. Which custom-call metadata is mandatory for deterministic review and judge review?
   How are secret arguments/results withheld without claiming complete semantic evidence?
5. How does registry identity interact with runtime reopen, worker pooling, and recovery?
6. What are the measured no-registry, Rust-callback, and Python-callback costs?
7. Is direct Pydantic AI domain-tool exposure required in addition to Monty mounting?
8. Does the resulting application remain a short, readable file with the business
   handler visible, or has framework setup overwhelmed the original use case?

### Resume procedure

Read sections 1–5 first. Confirm stabilization and inspect the source map against the
then-current worktree. Choose two domain fixtures, run the design spike, update this
document with verified decisions, and only then select public API names and begin
implementation. Keep v0 and deferred stages distinct in code, examples, and release
notes so the preserved idea never becomes an accidental promise of shipped behavior.
