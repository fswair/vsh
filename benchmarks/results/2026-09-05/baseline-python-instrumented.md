# VSH native core / PyO3 benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | boundary p50 µs | boundary p50 % |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.161 | 0.218 | 0.104 | 0.037 | 6.3 | 3.93% |
| read_10 | auto_approved | 0 | 0.937 | 7.473 | 0.115 | 0.783 | 9.2 | 0.98% |
| edit_20 | auto_approved | 20 | 1.880 | 3.202 | 0.107 | 1.110 | 8.8 | 0.47% |
| search_10k | auto_approved | 0 | 70.494 | 154.800 | 34.346 | 35.512 | 928.0 | 1.32% |
| vsh_glob_10k | auto_approved | 0 | 73.275 | 128.463 | 33.125 | 36.519 | 1443.7 | 1.97% |
| rename_subtree_100 | pending_approval | 202 | 56.947 | 65.044 | 32.114 | 0.287 | 956.0 | 1.68% |
| delete_subtree_100 | pending_approval | 101 | 47.321 | 52.002 | 32.323 | 1.859 | 956.2 | 2.02% |
| vsh_remove_subtree_100 | pending_approval | 101 | 46.928 | 54.237 | 32.281 | 0.546 | 968.7 | 2.06% |
| massive_delete_5k | pending_approval | 5050 | 167.011 | 208.518 | 32.183 | 96.990 | 1617.5 | 0.97% |

Repeated cold runtime-open p50/p99: 28.187/61.879 ms; first-call p50/p99: 4.387/4.763 ms.

Independent-runtime parallel speedup: 3.39x across 4 workers.
