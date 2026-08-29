# VSH native core / PyO3 benchmark

| Case | wall p50 ms | wall p99 ms | boundary p50 µs | boundary p50 % |
|---|---:|---:|---:|---:|
| noop | 0.190 | 0.533 | 9.5 | 4.99% |
| read_10 | 0.988 | 1.463 | 11.2 | 1.13% |
| edit_20 | 1.882 | 2.444 | 14.3 | 0.76% |

Independent-runtime parallel speedup: 2.72x across 4 workers.
