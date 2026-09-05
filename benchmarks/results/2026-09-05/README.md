# 2026-09-05 optimization evidence

## Baseline provenance

Working tree based on `b6380dc`, with the existing, uncommitted Monty 0.0.22 active
snapshot integration. No runtime optimization had been applied at baseline capture.
Host: macOS 26.1, Apple arm64, 8 logical CPUs; Python 3.14.6. Both SDK harnesses use
40 warm samples per case, one discarded warmup, 20 cold samples, four independent
runtimes in the concurrency case, and a 10,000-file / 100-directory fixture.

`baseline-rust.json` is the optimized release native baseline. `baseline-python.json`
is an initial **debug-extension diagnostic**, not a release baseline: its extension
SHA-256 equals `target/debug/lib_native.dylib`. Do not attribute the debug-to-release
difference to algorithmic optimization. `baseline-release-python.json` was captured
with the release extension before runtime changes and is the authoritative Python baseline.

Original artifact SHA-256 values:

- Python debug extension: `7578d8403c2ba2ab6ae3f2413a25ba4e515cfb0a7a792444d7016fcc12044c0b`
- Python release baseline extension: `d6a1378d915f8fd5b8eda87e0c47af006a95e7eaeeec066b0777dbb43a39236d`
- Release worker: `5ab8604f92bf502bb3672309181eca431990604c0c5c551419714cc79e3bdfc4`
- Release native harness: `8723281464125827686a6541d9a87eafb8379c72a57cf2d3f193a7c81506b436`
- Cargo.lock: `4b0c8f78388155e28a8ec4f168f8d3598ef77e5dc1d612102ab5747ec804ca05`

The native harness is owned by package `vsh-runtime` (`crates/vbash`), whose library
name is `vsh`; the public facade package is also named `vsh`. The worker package is
`vsh-monty-worker`. Build with these package identities, not directory names.

`/usr/bin/time -l` diagnostic: Rust run 22.63 s elapsed, 9.97 s user, 8.85 s system,
39,714,816-byte maximum resident set size. Debug Python run 73.73 s elapsed, 60.90 s
user, 9.37 s system, 56,672,256-byte maximum resident set size. These command rusage
high-water values are **not** simultaneous whole-worker-tree RSS or unique memory.

## Optimization sequence

Final measured artifact SHA-256 values (before later documentation/test-only edits):

- Release extension: `00c9451ec8372fd87ee073a90ef5542df6a4a22bcfe03221ebd41ae2834d8087`
- Release worker: `cb5c83d620babbb9e959794dc1c7cba162db14280fdb98faf656722544de3b63`
- Native harness: `74bb92d95bc1d8bf832c85f813f526b08898d22c266a96e9f36126d0e0a5f322`
- Policy/path harness: `0a0677b04bb5cd673e2ca3c7480a8382be1c73d084a161e7f58a002548c8d064`
- Cargo.lock remained unchanged from the baseline hash above.

1. Baseline: existing Monty 0.0.22 integration, no new runtime optimization.
2. `optimized-*`: allocation-free cursor policy matcher, canonical VPath fast path,
   fused snapshot validation/indexing, single tree insertion, empty-overlay resolution
   and a flat after-state buffer retaining two-phase lazy materialization.
3. `final-*`: additionally specialize leading-globstar basename rules, short-circuit
   trailing globstars, use borrowed canonical ancestor lookups, skip empty-overlay
   listing machinery and range-bound overlay child discovery with slash boundaries.
4. `confirmation-*`: independent repeat of the same final binary/workloads. These
   reports are not replacements selected to hide less favorable final samples.

Policy matching preserves the original first-denial order and source encoding. Snapshot,
diff and transaction digests, lazy identity checks, protected-access ledger, byte/entry
budgets, revalidation and durable commit ordering are unchanged. No dependency was added
or upgraded for this optimization. The only additive value-type surface is `Borrow<str>`
for canonical `VPath`, consistent with its existing string ordering.

## Reproduce the comparison

```bash
uv run --no-sync python benchmarks/compare.py \
  --results benchmarks/results/2026-09-05 \
  --output benchmarks/results/2026-09-05/comparison.json
```

`comparison.json` and its Markdown sibling derive latency, confirmation, microbenchmark
and sampled memory values from the raw reports. The generator checks warm iteration
counts, baseline/final case names, transaction states and changed-path outcomes;
it does not validate every cold/fixture/memory sampling setting. `*-instrumented` reports are
only the workload run under `process_tree.py`, not latency evidence.

Memory sampling uses `ps` every 50 ms and sums the benchmark process and descendants.
It can miss brief worker overlap and double-count shared pages. Observed process maxima
differed (Rust 5→2; Python 5→7), so the raw summed-tree peaks do **not** establish a
repeatable memory improvement or regression. Python root RSS also varied. Fewer
temporary allocations are established by the algorithms, not a universal RSS claim.

These are local sequential measurements, not randomized trials, hosted-platform
validation, LLM billing, production p99 guarantees or a complete commit benchmark.
The no-op/small-I/O cases are sensitive to scheduling; the confirmation Rust run also
showed slower durable rename/delete cases. Those observations are retained explicitly.
