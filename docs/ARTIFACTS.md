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
- bounded Monty result, stdout, counters, and change detail.

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

Artifacts are content-addressed and decoded under size/cardinality/path limits. A
decoded transaction binding must match the requested record. State log frames are
checksummed; only incomplete EOF data is repairable, while a complete corrupt frame
fails closed.

Commit plans, journals, and markers are opened with no-follow identity validation.
Recovery leaves ambiguous ownership untouched and reports it to the host.

See [Transactions](guides/transactions.md), [Architecture](ARCHITECTURE.md), and the
full [Threat model](threat-model.md).
