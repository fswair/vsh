# Native Rust and Python parity contract

The PyO3 SDK is an adapter over the native crate. This document defines what “same
engine” means in tests and releases.

## Canonical input

A parity case fixes:

- workspace fixture and base digest,
- program bytes and code hash,
- normalized runtime configuration,
- intent,
- run mode and receipt detail,
- execution budget,
- deterministic clock/random fixtures where relevant.

## Canonical output

The comparison projection contains:

```text
decision
error kind (when present)
transaction digest
base snapshot id/digest
created/modified/deleted/renamed paths
read and write dependency kinds
risk flags
resource counters
commit and verification status
artifact identity/content hash
```

Wall-clock timings, process-local handles, human formatting, and language-specific stack
frames are excluded from equality but validated independently.

An auto-approved preview handle is process-local on both surfaces. Its bound artifact
becomes durable before reservation when the same live runtime promotes it. Pending
independent-approval artifacts are durable at preview completion.

## Required tests

- Rust direct call and Python PyO3 call produce equal projections.
- Invalid paths map to the same native error kind and Python exception class.
- Protected reads are denied before returning any bytes on both surfaces.
- Resource-limit failures produce no commit and the same failure kind.
- Panic/worker crash does not unwind through Python and leaves a recoverable state.
- Concurrent Python calls release the GIL and do not serialize independent runtimes.
- Compact/full receipts differ only in documented detail, not decisions or digests.
- Large payloads return the same artifact content hash without mandatory inline copies.
- Version metadata agrees across crate, extension, Python package, and built wheel.

## Boundary budget

Benchmarks measure a native Rust call and an equivalent PyO3 call around the same core
operation. Python boundary p50 overhead must remain within the plan budget, and p99 must
not introduce unexplained tail amplification. Bytes copied and Python objects allocated
are recorded for large-payload cases.
