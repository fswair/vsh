# VSH native Rust benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | Rust API envelope p50 µs |
|---|---|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.175 | 0.255 | 0.116 | 0.041 | 4.2 |
| read_10 | auto_approved | 0 | 0.905 | 1.076 | 0.112 | 0.771 | 5.1 |
| edit_20 | auto_approved | 20 | 1.837 | 2.401 | 0.110 | 1.088 | 5.4 |
| search_10k | auto_approved | 0 | 55.598 | 59.730 | 30.642 | 24.056 | 892.9 |
| vsh_glob_10k | auto_approved | 0 | 59.206 | 62.976 | 30.825 | 24.774 | 1402.5 |
| rename_subtree_100 | pending_approval | 202 | 56.076 | 62.993 | 30.820 | 0.266 | 958.1 |
| delete_subtree_100 | pending_approval | 101 | 46.025 | 52.020 | 30.650 | 1.629 | 938.2 |
| vsh_remove_subtree_100 | pending_approval | 101 | 44.979 | 48.021 | 30.869 | 0.479 | 953.5 |
| massive_delete_5k | pending_approval | 5050 | 149.242 | 164.896 | 30.742 | 85.325 | 1585.0 |

Repeated cold runtime-open p50/p99: 27.709/34.690 ms; first-call p50/p99: 4.311/5.014 ms.

Independent-runtime parallel speedup: 3.56x across 4 workers.
