# VSH native Rust benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | Rust API envelope p50 µs |
|---|---|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.194 | 0.246 | 0.133 | 0.046 | 4.3 |
| read_10 | auto_approved | 0 | 1.018 | 1.331 | 0.125 | 0.861 | 5.4 |
| edit_20 | auto_approved | 20 | 1.882 | 3.839 | 0.114 | 1.120 | 5.5 |
| search_10k | auto_approved | 0 | 50.677 | 60.353 | 31.754 | 17.692 | 910.3 |
| vsh_glob_10k | auto_approved | 0 | 57.591 | 70.470 | 35.348 | 18.267 | 1460.9 |
| rename_subtree_100 | pending_approval | 202 | 66.967 | 79.559 | 36.376 | 0.266 | 1045.7 |
| delete_subtree_100 | pending_approval | 101 | 51.947 | 90.621 | 34.614 | 1.538 | 1030.8 |
| vsh_remove_subtree_100 | pending_approval | 101 | 50.073 | 81.860 | 34.131 | 0.343 | 1020.4 |
| massive_delete_5k | pending_approval | 5050 | 138.012 | 156.749 | 34.841 | 70.469 | 1663.5 |

Repeated cold runtime-open p50/p99: 46.911/95.278 ms; first-call p50/p99: 4.802/6.926 ms.

Independent-runtime parallel speedup: 3.60x across 4 workers.
