# VSH native core / PyO3 benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | boundary p50 µs | boundary p50 % |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.153 | 0.192 | 0.101 | 0.036 | 6.2 | 4.07% |
| read_10 | auto_approved | 0 | 0.886 | 1.232 | 0.108 | 0.752 | 8.3 | 0.94% |
| edit_20 | auto_approved | 20 | 1.769 | 2.301 | 0.103 | 1.060 | 8.2 | 0.46% |
| search_10k | auto_approved | 0 | 49.045 | 53.562 | 30.464 | 17.661 | 912.0 | 1.86% |
| vsh_glob_10k | auto_approved | 0 | 51.775 | 52.548 | 29.800 | 18.311 | 1374.4 | 2.65% |
| rename_subtree_100 | pending_approval | 202 | 55.922 | 61.963 | 30.844 | 0.268 | 1051.1 | 1.88% |
| delete_subtree_100 | pending_approval | 101 | 46.904 | 47.975 | 31.104 | 1.505 | 1036.8 | 2.21% |
| vsh_remove_subtree_100 | pending_approval | 101 | 45.049 | 55.950 | 30.797 | 0.348 | 1045.4 | 2.32% |
| massive_delete_5k | pending_approval | 5050 | 133.015 | 153.895 | 30.745 | 70.438 | 1662.9 | 1.25% |

Repeated cold runtime-open p50/p99: 41.594/106.027 ms; first-call p50/p99: 4.996/7.465 ms.

Independent-runtime parallel speedup: 2.87x across 4 workers.
