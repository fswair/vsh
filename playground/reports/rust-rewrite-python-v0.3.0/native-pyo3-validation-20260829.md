# VSH native core / PyO3 benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | boundary p50 µs | boundary p50 % |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.151 | 0.197 | 0.106 | 0.030 | 6.5 | 4.32% |
| read_10 | auto_approved | 0 | 0.837 | 1.277 | 0.103 | 0.714 | 6.9 | 0.83% |
| edit_20 | auto_approved | 20 | 1.797 | 4.005 | 0.099 | 1.071 | 7.0 | 0.39% |
| search_10k | auto_approved | 0 | 66.621 | 68.172 | 31.598 | 34.102 | 868.0 | 1.30% |
| rename_subtree_100 | pending_approval | 202 | 56.944 | 63.034 | 31.763 | 0.248 | 922.0 | 1.62% |
| delete_subtree_100 | pending_approval | 101 | 46.996 | 49.937 | 31.800 | 1.709 | 916.0 | 1.95% |
| massive_delete_5k | pending_approval | 5050 | 165.026 | 187.266 | 32.136 | 93.496 | 1497.7 | 0.91% |

Repeated cold runtime-open p50/p99: 34.055/84.540 ms; first-call p50/p99: 6.647/7.764 ms.

Independent-runtime parallel speedup: 4.20x across 4 workers.
