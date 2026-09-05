# Architecture

VSH is a transactional filesystem simulator with one Rust semantic core. Native Rust
enters through the `vsh` facade; Python/PyO3, CLI, and MCP reach the same
`vsh-runtime` implementation.

## System shape

```text
Rust application ───────────────────────────────┐
                                                │
Python application ── PyO3 typed adapter ───────┤
CLI / MCP ── bounded Python envelope ── PyO3 ───┤
                                                ▼
                                         vsh-runtime
                       ┌────────────────────────┼───────────────────────┐
                       ▼                        ▼                       ▼
                snapshot + VFS          Monty supervisor       policy + state
                       │                        │                       │
                       └──────────── canonical transaction ────────────┘
                                                │
                                                ▼
                              dependency revalidation + committer
                                                │
                                                ▼
                                      verified host receipt
```

## Core invariants

### One source of semantics

The Rust facade owns simulation, decision, state transition, durable artifacts,
revalidation, commit, and recovery. Python owns argument/result conversion and maps
typed errors to an exception hierarchy. MCP adds a bounded JSON-safe representation.

### No host mount in the guest

Monty receives a synthetic absolute `/workspace` namespace. Filesystem operations are
typed calls handled by the Rust adapter against `VirtualFs`. Ten high-level VSH
functions use the same typed suspension boundary and active overlay. There is no
guest-visible host path, subprocess, network, or ambient environment capability.

### Preview does not apply user-file changes

Snapshot and copy-on-write effects remain virtual until a transaction owns a valid
reservation and passes dependency/capability revalidation. Policy decisions and guest
return values cannot write the host.

Runtime storage is different: opening performs recovery, lazy capture stores immutable
blobs, and pending approvals persist evidence. Preview is not a promise of zero disk
writes anywhere on the host.

### Commit has one owner

Only `vsh-commit` applies host mutations. It receives a canonical plan and exact
preconditions, journals progress, verifies results, and makes interrupted work
recoverable. Adapters cannot implement alternate writers.

## Execution path

### Runtime open

1. Resolve and pin the workspace capability.
2. Establish a protected internal or trusted external data-directory capability.
3. Reject workspace/data overlap and symlink redirection.
4. Open bounded blob and transaction stores.
5. Recover incomplete trusted commit state.
6. Validate and initialize supervised worker configuration.

### Run

1. Validate source and request limits.
2. Capture a bounded immutable base snapshot.
3. Execute exact source in a supervised Monty worker.
4. Serve typed OS calls and high-level VSH functions through policy into the same
   copy-on-write `VirtualFs`.
5. Freeze an ordered canonical diff and dependency set.
6. Evaluate deterministic transaction policy.
7. Bind identity and retain/persist the exact pending artifact.
8. Return a receipt, or continue into auto commit when authorized.

Every run creates a new base and overlay. Metadata capture traverses the whole configured
workspace, excluding root `.vsh-runtime`; content is lazy and stamp-verified on capture.
There is no implicit `.gitignore` filter or retained preview overlay between calls.

### Commit

1. Persist an auto-approved process-local artifact if necessary.
2. Consume the single-use reservation.
3. Revalidate workspace/runtime identity and every bound dependency.
4. Write durable intent and canonical plan.
5. Apply capability-rooted operations with journal checkpoints.
6. Verify affected host nodes.
7. Mark committed and clean recoverable temporary state.

## Crate boundaries

| Crate | Boundary |
|---|---|
| `vsh-types` | Canonical values, digests, paths, and lifecycle legality |
| `vsh-vfs` | Snapshot representation and virtual effect semantics |
| `vsh-policy` | Call authorization and deterministic transaction decision |
| `vsh-monty` | Guest protocol, worker supervision, values, and budgets |
| `vsh-store` | Immutable blobs, approval grants, and atomic records |
| `vsh-commit` | Revalidation, host mutation, verification, recovery |
| `vsh` | Primary public SDK facade |
| `vbash` | Implementation-free compatibility re-export of `vsh` |
| `vsh-runtime` | Orchestration implementation and receipts |
| `vsh-python` | Thin PyO3 conversion/error boundary |
| `vsh-monty-worker` | Exact-version crash-isolated execution binary |

See the [crate map](rust/crates.md) for package names and dependency guidance.

## Concurrency model

- No global runtime execution lock exists.
- Each runtime bounds idle clean worker reuse; the host must separately limit active requests.
- Snapshot/simulation/policy work does not hold the commit coordination lock.
- Same-workspace commits serialize the minimal identity recheck and mutation window.
- Independent workspace runtimes scale in parallel.
- Python releases the GIL during native open, run, preview, commit, and recovery.

## Durability model

Transaction state transitions are compare-and-swap operations. Approval grants bind a
principal and time interval. A reservation is consumed once. Blob IDs and checksummed
frames detect corruption and binding mismatch.

Commit recovery distinguishes:

- safe finalization of an already-applied verified operation;
- safe rollback to original state;
- cleanup of owned temporary state;
- orphan/conflict where ownership cannot be proven.

The last category is reported, not guessed away.

## Performance model

The hot preview path avoids durable fsync only for deterministic auto-approved
artifacts, and only inside a hard-capped process-local cache. Approval-required work is
durable immediately. Promotion moves exact bytes into durable storage before consuming
the reservation.

Python performs one PyO3 call for a whole transaction and uses a typed result converter.
The remaining warm cost is dominated by real snapshot traversal and Monty/VirtualFs
work. See [Performance](performance.md).

The September 5 optimization removes policy/path allocation churn and repeated VFS
index work. It uses constant-space policy matching, canonical borrowed path lookups,
empty-overlay shortcuts, slash-bounded overlay ranges and flat two-phase diff buffers.
It does not cache away fresh snapshots or remove integrity/durability checks.

## Public contract

High-level callers should use `Runtime`, `RunRequest`, `Receipt`, and typed errors. The
facade re-exports lower-level configuration and evidence types, but applications should
not assemble an alternative commit pipeline.

- [Python SDK](python/)
- [Rust SDK](rust/)
- [MCP server](integrations/mcp.md)
- [Guarantees](rust-rewrite/GUARANTEES.md)
