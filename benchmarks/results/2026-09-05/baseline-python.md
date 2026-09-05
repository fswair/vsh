# VSH native core / PyO3 benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | boundary p50 µs | boundary p50 % |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.454 | 0.506 | 0.247 | 0.068 | 33.1 | 7.29% |
| read_10 | auto_approved | 0 | 1.747 | 2.223 | 0.257 | 1.261 | 38.2 | 2.19% |
| edit_20 | auto_approved | 20 | 3.370 | 3.834 | 0.259 | 1.898 | 44.7 | 1.33% |
| search_10k | auto_approved | 0 | 319.466 | 366.476 | 95.941 | 218.532 | 2835.6 | 0.89% |
| vsh_glob_10k | auto_approved | 0 | 371.985 | 432.655 | 94.978 | 244.284 | 3982.8 | 1.07% |
| rename_subtree_100 | pending_approval | 202 | 127.668 | 138.638 | 93.512 | 1.087 | 2900.0 | 2.27% |
| delete_subtree_100 | pending_approval | 101 | 118.949 | 123.994 | 93.479 | 6.599 | 2856.2 | 2.40% |
| vsh_remove_subtree_100 | pending_approval | 101 | 115.855 | 133.640 | 93.580 | 2.866 | 2886.4 | 2.49% |
| massive_delete_5k | pending_approval | 5050 | 659.292 | 733.452 | 93.675 | 400.068 | 4448.5 | 0.67% |

Repeated cold runtime-open p50/p99: 28.169/64.000 ms; first-call p50/p99: 4.951/5.930 ms.

Independent-runtime parallel speedup: 2.90x across 4 workers.
