# VSH native Rust benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | Rust API envelope p50 µs |
|---|---|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.161 | 0.260 | 0.108 | 0.037 | 3.8 |
| read_10 | auto_approved | 0 | 0.912 | 1.219 | 0.114 | 0.783 | 4.8 |
| edit_20 | auto_approved | 20 | 1.871 | 2.314 | 0.113 | 1.091 | 5.5 |
| search_10k | auto_approved | 0 | 49.300 | 52.783 | 30.747 | 17.761 | 922.1 |
| vsh_glob_10k | auto_approved | 0 | 52.410 | 54.395 | 30.387 | 18.348 | 1441.8 |
| rename_subtree_100 | pending_approval | 202 | 56.071 | 59.058 | 31.300 | 0.264 | 1037.5 |
| delete_subtree_100 | pending_approval | 101 | 46.857 | 54.088 | 31.359 | 1.513 | 1022.0 |
| vsh_remove_subtree_100 | pending_approval | 101 | 44.957 | 46.961 | 31.310 | 0.339 | 1014.6 |
| massive_delete_5k | pending_approval | 5050 | 131.489 | 147.423 | 31.423 | 70.968 | 1633.3 |

Repeated cold runtime-open p50/p99: 28.130/54.183 ms; first-call p50/p99: 4.632/5.417 ms.

Independent-runtime parallel speedup: 3.74x across 4 workers.
