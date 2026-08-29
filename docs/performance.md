# Performance

VSH is designed so the Python boundary and safety machinery do not become the next
bottleneck after removing the legacy Python engine. The retained benchmark record uses
the public Rust facade and an isolated CPython wheel over the same optimized worker.

Captured on 2026-08-29; values below are local macOS arm64 evidence, not universal
service-level objectives.

## Post-hardening warm latency

The final validation includes capability-rooted storage, identity rechecks, bounded
streaming reads, and fail-closed recovery handling.

| Workload | Rust p50 | PyO3 p50 | Estimated incremental PyO3 p50 |
|---|---:|---:|---:|
| No-op preview | 0.279 ms | 0.151 ms | within run noise |
| Read 10 files | 0.870 ms | 0.837 ms | 2.4 µs |
| Edit 20 files | 1.817 ms | 1.797 ms | 2.7 µs |
| Search 10,000 files | 66.918 ms | 66.621 ms | 9.7 µs |
| Rename 100-file subtree | 57.965 ms | 56.944 ms | 8.5 µs |
| Delete 100-file subtree | 47.057 ms | 46.996 ms | 16.5 µs |
| Delete 5,050 nodes | 174.850 ms | 165.026 ms | 1.3 µs |

The two harnesses run in separate processes, so their end-to-end wall medians are not
subtracted. The PyO3 estimate compares each surface's wall-minus-native-internal API
envelope. This avoids manufacturing a binding win from cross-process noise.

## Scaling

Four independent runtimes reached:

- **4.15×** speedup through the native Rust surface;
- **4.20×** speedup through Python/PyO3.

The throughput case amortizes thread creation. It demonstrates that worker pooling,
GIL release, and identity hardening did not introduce a process-global serialization
point. It does not promise linear scaling on every storage device or shared workspace.

## Cold start

In the primary retained 100-sample record, 30 independent cold runtimes measured:

| Stage | Native p50 / p99 | Python p50 / p99 |
|---|---:|---:|
| Runtime open | 21.2 / 26.1 ms | 17.5 / 26.6 ms |
| First worker call | 4.85 / 5.40 ms | 4.68 / 6.20 ms |

Cold values are reported separately from warm percentiles. Long-lived hosts should
reuse a runtime; process-per-call deployment pays startup and discards the bounded
worker pool.

## Driver-process memory

`/usr/bin/time -l` over the seven-case matrix reported:

| Surface | Maximum resident set | Peak memory footprint |
|---|---:|---:|
| Native Rust | 25.47 MiB | 20.69 MiB |
| CPython 3.14 + PyO3 | 45.80 MiB | 34.34 MiB |

These are driver-process high-water marks. They do **not** sum supervised worker
processes and therefore are not a whole-tree memory ceiling.

## Bottlenecks removed

### Durable write on every preview

The first implementation synchronously persisted every auto-approved preview. No-op,
10-file read, and 20-file edit medians were 11.896, 12.991, and 14.010 ms, with more
than 95% of no-op time in bind/store fsync work.

A bounded process-local cache now retains exact auto-approved previews. Promotion first
persists the artifact, then reserves, revalidates, and commits. Approval-required work
remains durable at preview completion. Development-wheel latency improved about 24.6×,
5.2×, and 3.4× for those three cases without relaxing commit ordering.

### Root-relative metadata resolution

The first 10,000-file search spent 257.8 ms of 293.2 ms in snapshot capture. Unix
snapshot traversal now reads no-follow metadata from already-open parent capabilities.
Snapshot p50 fell to 31.2 ms (8.3×), and end-to-end search fell to 66.9 ms (4.4×).

## Reproduce

```bash
export VSH_WORKER="$PWD/.venv/bin/vsh-monty-worker"

cargo build --release --locked -p vsh-runtime --example native_benchmark
target/release/examples/native_benchmark \
  --iterations 100 \
  --cold-iterations 30 \
  --parallel-workers 4 \
  --worker "$VSH_WORKER" \
  --output native-rust.json

python benchmarks/native_pyo3.py \
  --iterations 100 \
  --cold-iterations 30 \
  --parallel-workers 4 \
  --output native-pyo3.json
```

The harnesses have no third-party benchmark dependency. Compare runs from the same
machine state and release profile; do not subtract unrelated Rust and Python wall
samples.

## Remaining evidence

Hosted supported-platform runs still need whole worker-tree RSS, frozen legacy-Python
comparison, and adversarial stale/symlink/double-execute performance artifacts. Those
are release gates, not claims hidden from this page.

The complete methodology and raw-artifact paths are retained in the
[performance record](rust-rewrite/PERFORMANCE.md).
