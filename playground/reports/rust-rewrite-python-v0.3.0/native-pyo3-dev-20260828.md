# VSH native core / PyO3 benchmark

| Case | wall p50 ms | wall p99 ms | boundary p50 µs | boundary p50 % |
|---|---:|---:|---:|---:|
| noop | 0.483 | 0.943 | 35.1 | 7.27% |
| read_10 | 2.492 | 3.283 | 66.4 | 2.66% |
| edit_20 | 4.093 | 8.758 | 77.0 | 1.88% |

Independent-runtime parallel speedup: 2.32x across 4 workers.
