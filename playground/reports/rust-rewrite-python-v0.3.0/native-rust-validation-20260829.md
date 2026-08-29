# VSH native Rust benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | Rust API envelope p50 µs |
|---|---|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.279 | 0.415 | 0.197 | 0.059 | 6.5 |
| read_10 | auto_approved | 0 | 0.870 | 1.989 | 0.109 | 0.741 | 4.5 |
| edit_20 | auto_approved | 20 | 1.817 | 3.993 | 0.101 | 1.076 | 4.3 |
| search_10k | auto_approved | 0 | 66.918 | 68.272 | 31.929 | 34.073 | 858.3 |
| rename_subtree_100 | pending_approval | 202 | 57.965 | 62.016 | 32.129 | 0.248 | 913.5 |
| delete_subtree_100 | pending_approval | 101 | 47.057 | 54.056 | 32.115 | 1.725 | 899.5 |
| massive_delete_5k | pending_approval | 5050 | 174.850 | 207.850 | 32.345 | 93.605 | 1496.4 |

Repeated cold runtime-open p50/p99: 37.475/62.849 ms; first-call p50/p99: 8.651/10.174 ms.

Independent-runtime parallel speedup: 4.15x across 4 workers.
