# VSH native core / PyO3 benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | boundary p50 µs | boundary p50 % |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.127 | 0.159 | 0.078 | 0.031 | 7.0 | 5.47% |
| read_10 | auto_approved | 0 | 0.647 | 0.883 | 0.073 | 0.553 | 7.2 | 1.11% |
| edit_20 | auto_approved | 20 | 1.224 | 1.609 | 0.072 | 0.765 | 7.4 | 0.61% |
| search_10k | auto_approved | 0 | 66.594 | 67.905 | 31.251 | 34.409 | 896.0 | 1.35% |
| rename_subtree_100 | pending_approval | 202 | 55.947 | 58.204 | 32.281 | 0.265 | 995.8 | 1.78% |
| delete_subtree_100 | pending_approval | 101 | 47.835 | 49.852 | 32.688 | 1.752 | 1010.0 | 2.11% |
| massive_delete_5k | pending_approval | 5050 | 159.891 | 168.948 | 32.097 | 92.928 | 1573.5 | 0.98% |

Repeated cold runtime-open p50/p99: 17.533/26.582 ms; first-call p50/p99: 4.684/6.204 ms.

Independent-runtime parallel speedup: 2.71x across 4 workers.
