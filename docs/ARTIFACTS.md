# Transaction artifacts

VSH retains the minimum bounded evidence required to promote, reject, recover, and
audit a filesystem transaction.

## Bound identity

An artifact binds:

- program digest and optional intent digest;
- base snapshot and runtime configuration;
- read and write dependency digests;
- canonical diff digest;
- deterministic policy digest and decision;
- language/profile, bounded tagged output, raw streams, counters and ordered effects;
- evidence completeness, risk/denial evidence and canonical review detail.

Changing any bound input produces another transaction identity.

## Retention classes

| Decision | Retention | Reason |
|---|---|---|
| Denied | Receipt/state evidence only | Cannot be committed |
| Auto-approved preview | Bounded process-local artifact | Avoid fsync on the common preview path |
| Pending approval | Durable immutable blob + state | Must survive reviewer delay and restart |
| Promoted auto-approved preview | Persisted before reservation | Recovery requires exact bytes before mutation |
| Committed/recovery-required | Durable state, plan, journal, marker as needed | Verification and crash recovery |

Process-local retention is bounded by both artifact count and aggregate encoded bytes.
Defaults are 64 auto-approved artifacts or 128 MiB encoded bytes. Call
`discard_preview` for abandoned handles and completed read-only previews. It releases
process-local retention, not a general blob-store garbage collection or durable
pending-artifact cancellation. Restart or MCP runtime-LRU eviction loses auto-approved
handles; approval-required artifacts are durable.

## Integrity and recovery

Current pending artifacts use `VSHPND04`. Every fresh execution gets an OS-entropy
invocation identity separate from its semantic diff/evidence digests. Repeating the
same read or no-op therefore creates a fresh transaction; replaying one existing
approval or commit handle still fails. A persisted hook-configuration digest prevents
reopening the artifact with a missing or changed hook to bypass review.

Pending artifacts from older formats require a fresh preview before approval or commit.
Entered commits still use their journaled recovery protocol; do not delete recovery
state during an upgrade. Monty and Bash have separate output tags;
Bash records its profile, successful exit status and byte-authoritative stdout/stderr.
Loading recomputes the execution-evidence seal and identity. Unknown tags, incomplete
outputs, nonzero Bash exits, malformed lengths or combined output over the configured
ceiling are rejected before admission. Timings and display detail do not redefine
approval identity. Fresh artifacts are encoded and sealed in one bounded pass;
loading or promoting an existing artifact verifies its seal and never silently re-signs it.

Artifacts are content-addressed and decoded under size/cardinality/path limits. A
decoded transaction binding must match the requested record. State log frames are
checksummed; only incomplete EOF data is repairable, while a complete corrupt frame
fails closed.

Commit plans, journals, and markers are opened with no-follow identity validation.
Recovery leaves ambiguous ownership untouched and reports it to the host.

See [Transactions](guides/transactions.md), [Architecture](ARCHITECTURE.md), and the
full [Threat model](threat-model.md).
