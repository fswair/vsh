# VSH native Rust benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | Rust API envelope p50 µs |
|---|---|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.267 | 0.334 | 0.175 | 0.068 | 6.6 |
| read_10 | auto_approved | 0 | 0.869 | 1.370 | 0.108 | 0.745 | 5.0 |
| edit_20 | auto_approved | 20 | 1.856 | 2.377 | 0.105 | 1.110 | 5.0 |
| search_10k | auto_approved | 0 | 67.922 | 71.049 | 32.048 | 34.893 | 937.8 |
| vsh_glob_10k | auto_approved | 0 | 71.975 | 88.644 | 31.805 | 36.445 | 1435.0 |
| rename_subtree_100 | pending_approval | 202 | 56.976 | 59.993 | 31.713 | 0.285 | 989.1 |
| delete_subtree_100 | pending_approval | 101 | 47.952 | 70.035 | 31.911 | 1.869 | 981.8 |
| vsh_remove_subtree_100 | pending_approval | 101 | 48.012 | 64.699 | 33.460 | 0.570 | 1021.5 |
| massive_delete_5k | pending_approval | 5050 | 175.077 | 214.803 | 33.301 | 101.734 | 1672.4 |

Repeated cold runtime-open p50/p99: 27.556/49.895 ms; first-call p50/p99: 4.533/5.943 ms.

Independent-runtime parallel speedup: 2.77x across 4 workers.
