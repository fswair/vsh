# VSH native core / PyO3 benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | boundary p50 µs | boundary p50 % |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.181 | 0.264 | 0.121 | 0.042 | 7.2 | 4.00% |
| read_10 | auto_approved | 0 | 0.861 | 1.173 | 0.104 | 0.736 | 7.4 | 0.86% |
| edit_20 | auto_approved | 20 | 1.840 | 2.374 | 0.105 | 1.088 | 7.9 | 0.43% |
| search_10k | auto_approved | 0 | 68.130 | 73.891 | 32.438 | 34.546 | 916.3 | 1.34% |
| vsh_glob_10k | auto_approved | 0 | 73.716 | 85.084 | 32.555 | 36.408 | 1452.8 | 1.97% |
| rename_subtree_100 | pending_approval | 202 | 58.011 | 63.026 | 33.211 | 0.294 | 1029.3 | 1.77% |
| delete_subtree_100 | pending_approval | 101 | 48.875 | 89.384 | 33.086 | 1.867 | 1010.6 | 2.07% |
| vsh_remove_subtree_100 | pending_approval | 101 | 47.801 | 55.696 | 33.317 | 0.544 | 1015.0 | 2.12% |
| massive_delete_5k | pending_approval | 5050 | 182.400 | 266.815 | 33.498 | 99.716 | 1647.8 | 0.90% |

Repeated cold runtime-open p50/p99: 28.775/65.826 ms; first-call p50/p99: 4.205/4.853 ms.

Independent-runtime parallel speedup: 2.70x across 4 workers.
