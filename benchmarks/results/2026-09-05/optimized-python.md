# VSH native core / PyO3 benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | boundary p50 µs | boundary p50 % |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.153 | 0.227 | 0.101 | 0.035 | 6.1 | 4.02% |
| read_10 | auto_approved | 0 | 0.860 | 1.158 | 0.103 | 0.730 | 7.6 | 0.89% |
| edit_20 | auto_approved | 20 | 1.913 | 3.788 | 0.124 | 1.134 | 12.8 | 0.67% |
| search_10k | auto_approved | 0 | 56.116 | 59.360 | 30.571 | 24.412 | 915.8 | 1.63% |
| vsh_glob_10k | auto_approved | 0 | 58.862 | 65.612 | 30.316 | 24.930 | 1368.6 | 2.33% |
| rename_subtree_100 | pending_approval | 202 | 55.842 | 64.275 | 30.480 | 0.265 | 967.9 | 1.73% |
| delete_subtree_100 | pending_approval | 101 | 45.613 | 48.999 | 30.244 | 1.621 | 941.7 | 2.06% |
| vsh_remove_subtree_100 | pending_approval | 101 | 44.050 | 51.973 | 30.230 | 0.412 | 955.0 | 2.17% |
| massive_delete_5k | pending_approval | 5050 | 153.842 | 222.073 | 30.912 | 90.523 | 1616.0 | 1.05% |

Repeated cold runtime-open p50/p99: 29.700/79.032 ms; first-call p50/p99: 4.433/9.453 ms.

Independent-runtime parallel speedup: 2.72x across 4 workers.
