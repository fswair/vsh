# Native core and PyO3 performance record

Captured: 2026-08-29\
Native artifact: `playground/reports/rust-rewrite-python-v0.3.0/native-rust-release-20260829.json`\
PyO3 artifact: `playground/reports/rust-rewrite-python-v0.3.0/native-pyo3-release-20260829.json`\
macOS RSS artifact: `playground/reports/rust-rewrite-python-v0.3.0/memory-macos-arm64-20260829.json`\
Historical development artifact: `playground/reports/rust-rewrite-python-v0.3.0/native-pyo3-dev-20260828.json`
Post-hardening native validation: `playground/reports/rust-rewrite-python-v0.3.0/native-rust-validation-20260829.json`\
Post-hardening PyO3 validation: `playground/reports/rust-rewrite-python-v0.3.0/native-pyo3-validation-20260829.json`

## Reproducible command

Both harnesses have no third-party benchmark dependency. The native harness calls the
public `vsh-runtime` facade directly; the Python harness calls the PyO3 extension from
an isolated CPython 3.14 environment installed from the built wheel. Both use the exact
release worker bundled in that wheel.

```bash
export VSH_WORKER="$PWD/.venv/bin/vsh-monty-worker"
cargo build --release --locked -p vsh-runtime --example native_benchmark
target/release/examples/native_benchmark \
  --iterations 100 \
  --cold-iterations 30 \
  --parallel-workers 4 \
  --worker "$VSH_WORKER" \
  --output native-rust-release-20260829.json

python benchmarks/native_pyo3.py \
  --iterations 100 \
  --cold-iterations 30 \
  --parallel-workers 4 \
  --output native-pyo3-release-20260829.json
```

The paired record uses the same fat-LTO release profile for the Rust example, PyO3
extension, and matching worker. The development artifact retains the earlier 30-sample
workload as a compiler-independent hot-path regression aid.

## Result after bottleneck closure

| Case | state / changed paths | Rust wall p50 / p99 | PyO3 wall p50 / p99 | Rust API envelope p50 | Python wall − internal p50 | estimated PyO3-only p50 |
|---|---|---:|---:|---:|---:|---:|
| no-op preview | auto / 0 | 0.138 / 0.202 ms | 0.127 / 0.159 ms | 4.4 µs | 7.0 µs | 2.6 µs |
| read 10 files | auto / 0 | 0.673 / 0.820 ms | 0.648 / 0.884 ms | 4.3 µs | 7.2 µs | 2.9 µs |
| edit 20 files | auto / 20 | 1.278 / 1.560 ms | 1.224 / 1.610 ms | 4.5 µs | 7.4 µs | 2.9 µs |
| search 10k files | auto / 0 | 66.397 / 67.346 ms | 66.594 / 67.905 ms | 880.5 µs | 896.0 µs | 15.5 µs |
| rename 100-file subtree | pending / 202 | 56.005 / 59.092 ms | 55.947 / 58.204 ms | 1011.9 µs | 995.8 µs | within run noise |
| delete 100-file subtree | pending / 101 | 47.916 / 51.657 ms | 47.835 / 49.852 ms | 1003.9 µs | 1010.0 µs | 6.1 µs |
| delete 5,050 nodes | pending / 5,050 | 161.563 / 208.407 ms | 159.891 / 168.949 ms | 1568.1 µs | 1573.5 µs | 5.4 µs |

The native API envelope is the direct Rust-call wall time minus the receipt's internal
timer. The Python equivalent contains that same envelope plus PyO3 request/result
conversion, so their p50 difference estimates PyO3-only cost. Because the two
harnesses run in separate processes, their end-to-end wall medians are intentionally
not subtracted from each other. Across the seven cases the estimated incremental PyO3
p50 cost is 2.6–15.5 µs where distinguishable and stays far below both the 10% and
100 µs acceptance ceilings. The large-tree API envelope is mostly native cleanup
outside the receipt timer, not binding conversion. No case shows greater than 15% PyO3
p99 amplification in the retained 100-sample pair. A repeated PyO3 run was used after
an earlier run contained two isolated scheduler/disk outliers; both raw retained
artifacts contain p50/p95/p99/min/max and sample count.

The throughput case amortizes thread creation over 20 transactions per runtime, 80
transactions total. Four independent native runtimes reached 3.53× speedup; the PyO3
surface reached 2.71× while releasing the GIL. Thirty independent cold runtimes report
runtime-open p50/p99 of 21.2/26.1 ms in native Rust and 17.5/26.6 ms through Python;
first-worker-call p50/p99 is 4.85/5.40 ms and 4.68/6.20 ms respectively. Cold values
are reported separately and are not folded into warm percentiles.

On macOS arm64, `/usr/bin/time -l` over the seven-case matrix reported driver-process
peak RSS of 26,705,920 bytes (25.47 MiB) for native Rust and 48,021,504 bytes
(45.80 MiB) for CPython 3.14 plus PyO3. This is a high-water mark for the calling
process, not the sum of independently supervised worker processes; the raw scope and
commands are retained in the memory artifact.

## Post-hardening regression validation

After capability-rooted data storage, workspace/runtime identity checks, bounded
streaming reads, and fail-closed journal handling were added, the paired 100-sample
matrix was rerun with 30 independent cold starts:

| Case | Rust p50 | PyO3 p50 | estimated incremental PyO3 p50 |
|---|---:|---:|---:|
| no-op preview | 0.279 ms | 0.151 ms | within run noise |
| read 10 files | 0.870 ms | 0.837 ms | 2.4 µs |
| edit 20 files | 1.817 ms | 1.797 ms | 2.7 µs |
| search 10k files | 66.918 ms | 66.621 ms | 9.7 µs |
| rename 100-file subtree | 57.965 ms | 56.944 ms | 8.5 µs |
| delete 100-file subtree | 47.057 ms | 46.996 ms | 16.5 µs |
| delete 5,050 nodes | 174.850 ms | 165.026 ms | 1.3 µs |

The Python boundary remains below both acceptance ceilings in every case. Four
independent runtimes reached 4.15x native and 4.20x PyO3 speedup, so the new identity
checks did not introduce a process-global serialization point. The retained primary
run remains the quieter performance record: this validation followed several long
LTO/coverage builds; the native no-op process in particular shows run-order/system
variance that is absent from the separately executed Python process. Small read/edit
medians remain above the primary record, concentrated in Monty execution and snapshot
filesystem time rather than FFI. This is recorded as variance, not presented as a win,
and cross-process wall medians are never subtracted to manufacture a binding number. A
clean hosted run against the frozen Python baseline remains a Phase 10 release gate.

## Bottleneck closure

The first run of the same benchmark showed no-op/read/edit p50 values of 11.896,
12.991, and 14.010 ms. Stage attribution placed more than 95% of the warm no-op in
`bind_and_store`: every auto-approved preview synchronously fsynced an immutable blob,
its directory, and the state log.

Auto-approved `Preview` now validates and bounds its exact artifact but retains it in a
hard-capped process-local cache. Promotion first persists the artifact and lifecycle
record, then reserves and revalidates it. Approval-required transactions remain durable
at preview completion. The benchmark explicitly discards each sampled preview after
timing so a long run measures work rather than retained handles. This reduced the same
dev-wheel cases by approximately 24.6×, 5.2×, and 3.4× without weakening
commit/recovery ordering. Release optimization lowers the final warm p50 values again
to the primary table above.

The first isolated 10k-file search smoke exposed a second bottleneck: snapshot p50 was
257.8 ms of a 293.2 ms transaction because each child metadata lookup re-resolved its
full capability-relative path from the workspace root. Snapshot capture now obtains
no-follow metadata directly from the already-open parent directory entry on Unix;
Windows retains the full-metadata path needed for stable file identity. This uses the
same exact-pinned `cap-std` API and does not add a dependency or relax symlink/race
checks. Snapshot p50 fell to 31.2 ms (8.3×) and end-to-end search to 66.9 ms (4.4×).
The final search is balanced between snapshot (46.6%) and actual Monty/VFS directory
search (51.9%), rather than dominated by path re-resolution overhead.

The expanded mutation matrix preserves the normal balanced-policy path rather than
weakening it for a benchmark: rename, subtree delete, and 5,050-node delete all finish
as `pending_approval`, so their totals include bounded durable artifact persistence.
The 5,050-node case stays inside the production one-second and 10,000 OS-call caps; its
native p50 is distributed across snapshot (32.7 ms), Monty/VFS execution (93.8 ms),
diff plus policy (16.7 ms), and bind/store (16.7 ms). No hidden per-operation FFI loop
is introduced: each workload is one Python-to-Rust transaction call.

The remaining warm cost is dominated by real snapshot and Monty/VFS work rather than
PyO3. The remaining release matrix requires equivalent frozen-Python-baseline cases,
supported-platform and whole-worker-tree RSS CI measurements, and performance artifacts
for the adversarial stale/symlink/double-execute and large-artifact cases that are
currently covered as correctness/security tests.
