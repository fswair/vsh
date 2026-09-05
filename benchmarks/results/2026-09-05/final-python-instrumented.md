# VSH native core / PyO3 benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | boundary p50 µs | boundary p50 % |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.158 | 0.204 | 0.103 | 0.037 | 6.4 | 4.07% |
| read_10 | auto_approved | 0 | 0.917 | 1.380 | 0.118 | 0.773 | 8.9 | 0.97% |
| edit_20 | auto_approved | 20 | 1.773 | 2.238 | 0.105 | 1.038 | 8.6 | 0.49% |
| search_10k | auto_approved | 0 | 49.017 | 52.729 | 30.182 | 17.812 | 906.5 | 1.85% |
| vsh_glob_10k | auto_approved | 0 | 54.558 | 56.595 | 31.593 | 19.066 | 1509.9 | 2.77% |
| rename_subtree_100 | pending_approval | 202 | 56.901 | 65.796 | 31.331 | 0.290 | 1019.2 | 1.79% |
| delete_subtree_100 | pending_approval | 101 | 45.937 | 50.008 | 30.811 | 1.513 | 1024.0 | 2.23% |
| vsh_remove_subtree_100 | pending_approval | 101 | 44.903 | 48.869 | 31.139 | 0.345 | 1040.8 | 2.32% |
| massive_delete_5k | pending_approval | 5050 | 134.194 | 177.616 | 31.244 | 70.754 | 1716.9 | 1.28% |

Repeated cold runtime-open p50/p99: 29.327/71.688 ms; first-call p50/p99: 4.643/5.085 ms.

Independent-runtime parallel speedup: 3.27x across 4 workers.
