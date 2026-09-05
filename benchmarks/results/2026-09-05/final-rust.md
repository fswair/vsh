# VSH native Rust benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | Rust API envelope p50 µs |
|---|---|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.167 | 0.257 | 0.110 | 0.041 | 4.0 |
| read_10 | auto_approved | 0 | 0.887 | 1.099 | 0.110 | 0.755 | 4.8 |
| edit_20 | auto_approved | 20 | 1.943 | 3.538 | 0.120 | 1.152 | 6.8 |
| search_10k | auto_approved | 0 | 49.002 | 51.612 | 30.621 | 17.429 | 886.4 |
| vsh_glob_10k | auto_approved | 0 | 54.002 | 60.993 | 30.729 | 19.765 | 1407.7 |
| rename_subtree_100 | pending_approval | 202 | 55.962 | 65.344 | 31.547 | 0.263 | 1031.2 |
| delete_subtree_100 | pending_approval | 101 | 46.048 | 48.000 | 31.549 | 1.473 | 1008.6 |
| vsh_remove_subtree_100 | pending_approval | 101 | 45.082 | 50.360 | 31.794 | 0.339 | 1018.2 |
| massive_delete_5k | pending_approval | 5050 | 131.162 | 148.494 | 31.490 | 69.890 | 1630.0 |

Repeated cold runtime-open p50/p99: 28.073/86.135 ms; first-call p50/p99: 4.724/5.592 ms.

Independent-runtime parallel speedup: 3.27x across 4 workers.
