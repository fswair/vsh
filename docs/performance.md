# Performance and optimization evidence

The 2026-09-05 release-profile measurements show roughly **25–30% lower median latency**
for the large filename-discovery, glob and bulk-delete workloads after this optimization.
The strongest repeatable gain is less CPU and allocation work inside execution—not
skipping snapshots, policy, dependency checks or durability.

These are local macOS arm64 results for the **VSH 0.4.0 Monty 0.0.22 implementation**,
not cross-platform guarantees or service-level objectives.

## Measurement protocol

- Host: macOS 26.1, Apple arm64, 8 logical CPUs; CPython 3.14.6.
- Both the native harness and PyO3 extension use optimized release builds and a matching
  supervised worker. No in-process execution shortcut.
- Per run: 40 retained warm samples per case after one discarded warmup; 20 independent
  cold-runtime samples; four independent runtimes in the concurrency case.
- Large fixture: 100 directories with 100 files each. Small fixture: 20 input files.
- Baseline captured **before** runtime optimization. Final and independent confirmation
  runs use the same workloads, counts, decisions and limits.
- Timed cases are previews, including durable pending-approval storage where required;
  they do not measure actual application/commit latency.

Raw JSON, environment details, intermediate results and caveats live in
[`benchmarks/results/2026-09-05/`](https://github.com/fswair/vsh/tree/main/benchmarks/results/2026-09-05).
The generated `comparison.json` / `comparison.md` include every case, p95 values,
confirmation runs and stage distributions. An initial **debug-extension diagnostic**
is retained but explicitly excluded from the release comparison.

## Warm preview latency

Milliseconds, p50. Before → final; negative outcomes have not been removed.

| Workload | Rust before → final | Python before → final |
|---|---:|---:|
| No-op | 0.267 → 0.167 | 0.181 → 0.287 |
| Read 10 files | 0.869 → 0.887 | 0.861 → 1.024 |
| Edit 20 files | 1.856 → 1.943 | 1.840 → 1.774 |
| Filter names in 10,000-file tree | 67.922 → 49.002 | 68.130 → 48.689 |
| `vsh_glob` in 10,000-file tree | 71.975 → 54.002 | 73.716 → 51.964 |
| Rename 100-file subtree | 56.976 → 55.962 | 58.011 → 55.977 |
| Remove 100-file subtree with typed OS calls | 47.952 → 46.048 | 48.875 → 45.981 |
| `vsh_remove` of 100-file subtree | 48.012 → 45.082 | 47.801 → 45.057 |
| Remove 5,000 files + 50 directories | 175.077 → 131.162 | 182.400 → 130.868 |

The name-filter case knows the fixture has two directory levels and returns a count.
The glob case uses generic recursive matching and returns 1,000 typed paths. Neither
is a text-content search benchmark, and they are not identical result contracts.

### Confirmation and tail behavior

An independent repeat of the optimized binary measured:

| Workload | Rust repeat p50 / p95 | Python repeat p50 / p95 |
|---|---:|---:|
| Filename filtering | 50.677 / 52.319 | 49.045 / 51.466 |
| Generic glob | 57.591 / 63.819 | 51.775 / 52.185 |
| Bulk delete preview | 138.012 / 147.617 | 133.015 / 138.733 |

The large-workload gain repeats, though its magnitude varies. Small calls are sensitive
to scheduling and filesystem state: Python's no-op/read repeat was 0.153/0.886 ms,
while the first final run was 0.287/1.024 ms. The Rust confirmation's durable rename
case also rose to 66.967 ms. **No general small-call, durable-I/O or tail-latency
improvement is claimed.** These sequential local runs are not randomized A/B trials.

## Where time was removed

Rust p50 stage times, milliseconds:

| Stage within workload | Before | Final |
|---|---:|---:|
| Filename-filter execution | 34.893 | 17.429 |
| Glob execution | 36.445 | 19.765 |
| Bulk-delete execution | 101.734 | 69.890 |
| Bulk-delete canonical diff | 5.657 | 4.321 |
| Bulk-delete final policy evaluation | 11.600 | 3.534 |
| Filename-filter snapshot | 32.048 | 30.621 |

Call-policy checks performed during traversal are charged to **execute**, not just the
final `policy` stage. Stage medians are measured independently and need not sum to the
wall-time median. Durable binding/storage remains real work; it was not bypassed.

### Algorithms and data structures

| Change | Removed work | Preserved contract |
|---|---|---|
| Cursor-based policy globstar matcher | Per-path component vectors and one DP allocation per pattern component | Same matching language, canonical rules and first denial |
| Leading-globstar basename specialization | Scanning parent components for `**/*.key`-style rules | Same root and nested-path semantics |
| Canonical `VPath` fast path | Separator replacement/vector/join for already canonical inputs | Portable normalization and rejection priority |
| Borrowed ancestor lookups | Allocating a new owned path for every ancestor probe | Tombstone and non-directory shadowing |
| Empty-overlay resolution/listing | Parent probes, temporary tree and redundant visibility lookups | Immutable base visibility |
| Slash-bounded overlay range | Scanning every unrelated overlay path for a directory listing | Exact component boundaries and sorted results |
| Fused snapshot indexing / entry insertion | Duplicate parent derivation and B-tree searches | Parent validation order and snapshot identity |
| Flat canonical-diff after-state buffer | Extra keyed tree and cloned path keys | Complete lazy after-materialization before reading before-states |

The policy matcher uses constant auxiliary memory and no recursion. It performs at
most O(pattern components × path components) component matches; wildcard work inside
each component is separate. Overlay child discovery visits the relevant lexical
subtree, not an unrelated whole overlay, and retains no extra permanent child index.

A separate 10,000-path microbenchmark, with 1,000 expected denials and ten retained
samples, measured policy authorization median **17.875 → 5.806 ms (67.5% lower)** and
canonical parsing **1.119 → 0.712 ms (36.4% lower)**. It isolates these costs; it is not
an end-to-end latency claim or an allocation-profiler trace.

Correctness checks compare the new matcher against the original dynamic-programming
oracle over 94,501 combinations, including empty paths and multiple globstars, plus
deep paths and compiled fast paths. Portable-path oracle checks, prefix-sibling listing
tests and generated VFS operation sequences protect normalization and diff semantics.

## CPU and memory costs

Command-reported user + system time for the complete harness, including fixture,
cold and concurrency work, fell from **18.82 to 15.27 seconds** for Rust and
**18.93 to 15.25 seconds** for Python. This is about 19% lower reported CPU time for
that matrix, not an isolated per-transaction CPU measurement or a billing estimate.

Separate process-tree RSS sampling every 50 ms recorded:

| Surface | Root peak MiB before → final | Summed tree peak MiB before → final | Max observed processes |
|---|---:|---:|---:|
| Rust | 36.31 → 33.42 | 56.09 → 42.22 | 5 → 2 |
| Python | 53.86 → 57.81 | 77.78 → 97.28 | 5 → 7 |

**These samples do not establish a repeatable RSS reduction or regression.** The
sampler missed different short-lived overlaps; summing RSS also double-counts shared
pages and is not unique memory/PSS. Python's sampled root peak increased despite
lower temporary allocation work. Command-reported RSS high-water marks use another
scope and are retained separately in `command-rusage.json`.

The implementation eliminates specific temporary allocations; a universal resident
memory win has not been demonstrated. No model calls, token prices, storage retention
costs or monetary savings were measured.

## Cold startup and parallel work

The final run's 20 independent cold samples measured runtime-open p50 of 28.07 ms
(Rust) and 28.62 ms (Python), followed by first-call p50 of 4.72/4.79 ms. Keep cold
startup separate from warm preview figures. Reuse runtimes when the trusted
workspace/configuration is stable.

Four independent runtimes measured 3.27× native and 3.30× Python throughput speedup
over the harness's sequential phase. This demonstrates useful concurrency in this
workload, not guaranteed linear scaling or same-workspace commit throughput.

## Reproduce

Build the current release extension and matching worker using [development](development.md).
Run the two surfaces **sequentially**, without simultaneous tests, compilers or memory
samplers:

```bash
cargo build --release --locked -p vsh-runtime --example native_benchmark
target/release/examples/native_benchmark \
  --iterations 40 --cold-iterations 20 --parallel-workers 4 \
  --worker "$PWD/target/release/vsh-monty-worker" --output native-rust.json

VSH_MONTY_WORKER="$PWD/target/release/vsh-monty-worker" \
  uv run --no-sync python benchmarks/native_pyo3.py \
  --iterations 40 --cold-iterations 20 --parallel-workers 4 --output native-python.json
```

For separate memory instrumentation:

```bash
uv run --no-sync python benchmarks/process_tree.py --output memory.json -- \
  target/release/examples/native_benchmark \
  --iterations 40 --cold-iterations 20 --parallel-workers 4 \
  --worker "$PWD/target/release/vsh-monty-worker" --output instrumented.json
```

Do not use `instrumented.json` as latency evidence. Rebuild and verify release artifacts
before comparing; the first diagnostic in this session caught a debug extension that
would otherwise have produced a misleading speedup claim.

## Remaining costs and deliberate boundaries

Fresh metadata traversal still costs about 30–31 ms on this large fixture. Reducing
the trusted workspace root is an immediate application-level lever. A TTL snapshot
cache would weaken freshness and was not introduced. Bulk typed-call loops still pay
IPC; use bounded compound functions where their semantics fit. Pending approval and
commit still pay real durable I/O and integrity/revalidation costs.

Hosted OS/storage measurements, controlled steady-state process memory, commit/recovery
performance and actual agent-loop cost remain separate evidence work. See
[efficient usage](guides/efficient-usage.md) for practical tuning.
