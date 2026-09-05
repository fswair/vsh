# VSH native core / PyO3 benchmark

| Case | state | changed paths | wall p50 ms | wall p99 ms | snapshot p50 ms | execute p50 ms | boundary p50 µs | boundary p50 % |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| noop | auto_approved | 0 | 0.287 | 0.330 | 0.182 | 0.072 | 11.5 | 4.02% |
| read_10 | auto_approved | 0 | 1.024 | 1.573 | 0.124 | 0.868 | 9.0 | 0.87% |
| edit_20 | auto_approved | 20 | 1.774 | 2.365 | 0.108 | 1.059 | 8.8 | 0.50% |
| search_10k | auto_approved | 0 | 48.689 | 50.019 | 30.163 | 17.561 | 895.8 | 1.84% |
| vsh_glob_10k | auto_approved | 0 | 51.964 | 54.445 | 30.264 | 18.185 | 1374.0 | 2.64% |
| rename_subtree_100 | pending_approval | 202 | 55.977 | 62.025 | 31.390 | 0.269 | 1048.1 | 1.87% |
| delete_subtree_100 | pending_approval | 101 | 45.981 | 48.882 | 31.402 | 1.510 | 1031.6 | 2.24% |
| vsh_remove_subtree_100 | pending_approval | 101 | 45.057 | 50.181 | 31.563 | 0.344 | 1038.9 | 2.31% |
| massive_delete_5k | pending_approval | 5050 | 130.868 | 149.062 | 31.025 | 70.156 | 1678.9 | 1.28% |

Repeated cold runtime-open p50/p99: 28.615/93.915 ms; first-call p50/p99: 4.795/5.076 ms.

Independent-runtime parallel speedup: 3.30x across 4 workers.
