# Threat model

## Security objective

Execute an untrusted Monty Python program against a virtual workspace, decide whether
its exact resulting transaction is allowed, and apply only that transaction through a
trusted committer without exposing unauthorized host data or mutation paths.

## Assets

- Workspace file integrity, confidentiality, names, metadata, and permissions.
- Protected files and secret material.
- Immutable snapshots, blobs, transaction digests, approval decisions, and receipts.
- Checksummed bounded transaction logs, immutable approval artifacts, and commit journal.
- Host resources: CPU, memory, descriptors, disk, output, and wall-clock time.
- Rust/Python API semantic parity.
- Dependency and release provenance.

## Trust boundaries

| Component | Trust level | Notes |
|---|---|---|
| User goal | trusted intent, untrusted data | It constrains approval but may contain hostile text. |
| Main agent and generated program | untrusted | It may deliberately attempt escape, exfiltration, or exhaustion. |
| Python caller/PyO3 inputs | untrusted | Python cannot grant capabilities or bypass native validation. |
| Monty interpreter/worker | isolated execution component | It receives no raw host mount and is discarded after unsafe termination. |
| Configured worker executable | trusted installation input | Exact Monty version is checked; choosing a replacement binary grants native-code authority. |
| VirtualFs and deterministic policy | trusted security core | Complete mediation and one filesystem semantics live here. |
| Fresh judge | advisory/separate principal | It cannot reverse deterministic deny. |
| Transaction store | trusted state | CAS transitions enforce single-use commit. |
| Committer | smallest trusted host writer | It alone may mutate the real workspace. |
| External concurrent process | adversarial race source | Revalidation must detect dependency changes. |
| Third-party dependencies | supply-chain risk | Exact pins and continuous advisory/license/source checks apply. |

## Attacker capabilities

Assume an attacker can:

- control Monty source code and input paths,
- use `..`, absolute paths, platform separators, symlinks, rename chains, and races,
- enumerate directories and condition outputs on secret data,
- request huge reads/writes/output or infinite computation,
- crash a worker or process at any commit boundary,
- replay, duplicate, reorder, or mutate transaction/approval identifiers,
- call public Rust or Python APIs with malformed values,
- trigger Python exceptions, finalization, threads, or interpreter shutdown,
- race the workspace between snapshot, approval, revalidation, and commit,
- exploit a vulnerable transitive dependency if one is present.

Assume the attacker cannot already execute arbitrary host-native code in the trusted VSH
process. If raw host shell/filesystem tools are simultaneously available, VSH is not a
complete security boundary.

## Primary threats and controls

### Host escape and confused paths

Threats: absolute paths, parent traversal, separator ambiguity, symlink/junction races,
rename escape, and time-of-check/time-of-use gaps.

Controls:

- validated `VPath` is relative and normalized,
- capability-rooted descriptor access in the committer,
- no path resolution fallback to ambient process CWD,
- symlink policy applied before data exposure and again at commit,
- adversarial platform/path tests.

### Confidentiality and output exfiltration

Threats: reading `.env`, keys, credentials, protected metadata, or encoding secrets into
stdout, errors, diffs, and receipts.

Controls:

- pre-call read capability policy,
- default-deny protected patterns,
- output and error payload budgets,
- compact receipts and artifact handles,
- redaction is defense-in-depth, not permission to perform the read.

### Virtual/host semantic divergence

Threats: simulator predicts one effect while a distinct executor performs another;
recursive and rename descendants are incompletely represented.

Controls:

- one VirtualFs implementation,
- canonical diff derived from final virtual state,
- committer applies the diff rather than replaying the original program,
- property tests against an independent filesystem model,
- post-commit expected-state verification.

### Stale approval and replay

Threats: approval reused for different bytes/state, double execution, concurrent token
use, or approval after workspace drift.

Controls:

- approval binds transaction digest, base snapshot, code hash, policy result, and diff,
- persisted state-machine CAS reservation,
- ReadSet/WriteSet revalidation,
- single-use commit transition,
- expiry where external approval is used.

### Partial commit and crash consistency

Threats: process death after staging, first rename, deletion, verification, state-log
compaction, or before transaction-state finalization leaves undefined host state.

Controls:

- staging/quarantine,
- durable commit journal,
- two alternating state logs selected by a fixed-size double-buffered control record,
- compacted log synchronization before the control generation can become active,
- idempotent recovery protocol,
- fault injection at every journal boundary,
- post-commit verification before final state.

### Resource exhaustion

Threats: infinite computation, recursive trees, giant output, many OS calls, descriptor
exhaustion, blob/overlay growth, and worker poisoning after a limit.

Controls:

- wall time, memory, calls, bytes, entries, output, and overlay budgets,
- bounded data structures and receipt sizes,
- per-runtime count and encoded-byte ceilings for process-local preview artifacts,
- discard workers after limit/panic,
- no unbounded queues or process-global serialization.

### PyO3 boundary failure

Threats: panic unwinds through FFI, GIL held during long work, borrowed Python data
outlives the GIL, Python exception interrupts transaction cleanup, or Python widens
capability configuration.

Controls:

- catch/map panic before the FFI boundary and fail closed,
- own/validate data before releasing the GIL,
- keep Python objects outside the native hot path,
- native transaction guard performs cleanup regardless of Python exception,
- validate Python-result compatibility before artifact persistence or reservation so a
  known conversion failure cannot occur after auto-commit,
- parity and concurrency tests through both language surfaces.

### Supply chain

Threats: abandoned crate, compromised release, known advisory, yanked version, malicious
feature/default dependency, source substitution, or unreviewed lockfile drift.

Controls:

- deny-by-default admission records,
- exact version pins and committed lockfile,
- crates.io/workspace sources only,
- minimal features,
- `cargo audit` and `cargo deny`,
- explicit dependency update reviews,
- release provenance and built-artifact tests.

## Security test families

- Host containment: absolute/parent/symlink/rename/mount escape.
- Destructive virtual execution: recursive delete changes only overlay before commit.
- Recursive correctness: final state and canonical diff include descendants exactly.
- Snapshot consistency: lazy reads become immutable or mark the transaction stale.
- Approval integrity: any transaction component change invalidates approval.
- Double commit: concurrent second reservation fails before host mutation.
- Worker crash: no host effect and worker is not reused.
- Secrets: denied bytes never reach Monty, Python, stdout, error, artifact, or receipt.
- Recovery: every injected crash boundary converges to a defined recoverable state.
- FFI parity: Rust/Python decisions, errors, receipts, and side effects match.

## Explicit non-goals

- Protecting a host where the same agent also has unrestricted shell/filesystem access.
- Detecting all malicious intent or proving user-request semantic correctness.
- Providing a full OS/container sandbox for network, processes, devices, or syscalls not
  exposed by the supported typed Monty surface.
- Guaranteeing that no undiscovered dependency vulnerability exists.
- Preventing a privileged host administrator from modifying VSH state.
