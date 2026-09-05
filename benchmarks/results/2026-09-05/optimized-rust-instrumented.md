# VSH native Rust benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | Rust API envelope p50 µs |
|---|---|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.153 | 0.219 | 0.103 | 0.035 | 3.8 |
| read_10 | auto_approved | 0 | 0.870 | 1.230 | 0.105 | 0.744 | 4.6 |
| edit_20 | auto_approved | 20 | 1.828 | 2.369 | 0.105 | 1.093 | 5.4 |
| search_10k | auto_approved | 0 | 55.577 | 60.520 | 30.491 | 24.322 | 905.2 |
| vsh_glob_10k | auto_approved | 0 | 58.822 | 64.447 | 30.467 | 24.878 | 1416.5 |
| rename_subtree_100 | pending_approval | 202 | 55.195 | 57.037 | 30.334 | 0.269 | 946.1 |
| delete_subtree_100 | pending_approval | 101 | 45.916 | 46.981 | 30.176 | 1.645 | 930.0 |
| vsh_remove_subtree_100 | pending_approval | 101 | 44.973 | 58.602 | 30.480 | 0.407 | 964.9 |
| massive_delete_5k | pending_approval | 5050 | 150.108 | 248.657 | 30.670 | 86.712 | 1572.2 |

Repeated cold runtime-open p50/p99: 29.644/32.255 ms; first-call p50/p99: 4.412/4.996 ms.

Independent-runtime parallel speedup: 3.52x across 4 workers.
