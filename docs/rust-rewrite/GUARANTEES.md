# VSH Rust rewrite guarantees and non-guarantees

## Guarantees

When VSH is the only filesystem authority available to the untrusted program and the
documented platform/configuration requirements are met, VSH guarantees:

- The program executes against virtual state before any host mutation.
- Supported filesystem operations share one Rust VirtualFs semantics.
- A deterministic deny cannot be overridden by an approval judge.
- Approval, when needed, binds to the exact transaction artifact.
- The same transaction cannot be committed twice.
- Commit revalidates every recorded content, metadata, directory, and write dependency.
- Stale, malformed, worker-crashed, over-budget, or failed-revalidation transactions
  do not reach host mutation.
- Only the trusted committer writes the host workspace.
- Successful commit is checked against the canonical expected final state.
- Protected reads are denied before bytes enter Monty or cross the PyO3 boundary.
- CPU/time/memory/call/read/write/output/overlay budgets are enforced and a failed
  worker is discarded.
- Rust and Python surfaces call the same core and must pass the same golden corpus.
- Python does not have a fallback execution engine.
- Model-facing results and output are bounded; durable transaction artifacts stay
  internal until a separately bounded artifact-read API is released.
- Process-local auto-approved preview retention has hard count and encoded-byte caps;
  capacity exhaustion fails before another preview handle is retained.

## Conditional guarantees

- Crash recovery requires the transaction database, journal, staging area, and target
  workspace to use supported durability/filesystem semantics.
- Atomicity across multiple files is implemented as recoverable journaled commit; it is
  not claimed to be a universal filesystem-level atomic multi-file transaction.
- A trusted-process or machine crash after commit reservation may interrupt a multi-file
  mutation; the durable journal and staging/quarantine evidence make it recoverable on
  the documented filesystems.
- Confidentiality applies to the typed VSH/Monty surface. It is void if the caller also
  exposes raw shell, unrestricted Python, another filesystem tool, or direct file
  descriptors to the untrusted program.
- Concurrency safety covers recorded dependencies. Unmodeled external side effects are
  outside the transaction.
- An auto-approved preview handle is resumable only by the same live `Runtime`. VSH
  persists its exact artifact before commit reservation; approval-required previews are
  durable immediately and may be resumed after restart.
- “No known vulnerability” is a timestamped registry/advisory result, not proof that no
  vulnerability exists.

## Non-guarantees

VSH does not guarantee:

- that an allowed edit is useful, correct, or desired by the user,
- that a fresh judge is infallible,
- protection against a compromised trusted VSH process or host administrator,
- arbitrary shell, subprocess, network, device, IPC, or kernel sandboxing,
- compatibility with every host filesystem or network filesystem,
- preservation of behavior that conflicts with the new security invariants,
- zero-copy for every Python value or platform,
- a specific latency on undeclared hardware,
- immunity from undiscovered CVEs,
- indefinite compatibility with unpinned external clients.

## Language parity contract

For identical runtime configuration, workspace base, program, intent, mode, and budget:

- Rust and Python return the same decision kind.
- Canonical diff, transaction digest, risk flags, and receipt fields are equivalent.
- Error categories are one-to-one; Python exceptions map from native error kinds.
- Both surfaces enforce the same resource and capability limits.
- A Python call cannot enable behavior unavailable to the native API.

Representation details may be idiomatic per language, but semantics may not diverge.
