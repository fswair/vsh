# VSH native Rust benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | Rust API envelope p50 µs |
|---|---|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.138 | 0.202 | 0.087 | 0.035 | 4.4 |
| read_10 | auto_approved | 0 | 0.673 | 0.820 | 0.076 | 0.582 | 4.3 |
| edit_20 | auto_approved | 20 | 1.278 | 1.560 | 0.075 | 0.802 | 4.5 |
| search_10k | auto_approved | 0 | 66.397 | 67.346 | 31.023 | 34.422 | 880.5 |
| rename_subtree_100 | pending_approval | 202 | 56.005 | 59.092 | 32.248 | 0.261 | 1011.9 |
| delete_subtree_100 | pending_approval | 101 | 47.916 | 51.657 | 32.510 | 1.765 | 1003.9 |
| massive_delete_5k | pending_approval | 5050 | 161.563 | 208.407 | 32.720 | 93.802 | 1568.1 |

Repeated cold runtime-open p50/p99: 21.150/26.094 ms; first-call p50/p99: 4.847/5.404 ms.

Independent-runtime parallel speedup: 3.53x across 4 workers.
