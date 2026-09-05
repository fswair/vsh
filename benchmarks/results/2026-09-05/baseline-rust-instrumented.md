# VSH native Rust benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | Rust API envelope p50 µs |
|---|---|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.154 | 0.201 | 0.105 | 0.036 | 3.8 |
| read_10 | auto_approved | 0 | 0.888 | 1.066 | 0.108 | 0.754 | 5.0 |
| edit_20 | auto_approved | 20 | 2.011 | 7.984 | 0.125 | 1.202 | 6.2 |
| search_10k | auto_approved | 0 | 68.778 | 78.012 | 32.996 | 34.951 | 919.1 |
| vsh_glob_10k | auto_approved | 0 | 73.762 | 87.663 | 33.045 | 36.707 | 1469.7 |
| rename_subtree_100 | pending_approval | 202 | 57.941 | 61.868 | 32.501 | 0.288 | 963.8 |
| delete_subtree_100 | pending_approval | 101 | 48.042 | 53.874 | 32.383 | 1.877 | 950.5 |
| vsh_remove_subtree_100 | pending_approval | 101 | 46.150 | 56.963 | 32.659 | 0.546 | 970.8 |
| massive_delete_5k | pending_approval | 5050 | 167.345 | 183.510 | 32.595 | 96.712 | 1590.0 |

Repeated cold runtime-open p50/p99: 27.232/38.323 ms; first-call p50/p99: 4.520/5.718 ms.

Independent-runtime parallel speedup: 2.37x across 4 workers.
