# Optimization comparison

## Rust release

| Case | Before p50 ms | Final p50 ms | Change | Final p95 ms | Repeat p50 ms |
|---|---:|---:|---:|---:|---:|
| noop | 0.267 | 0.167 | 37.3% lower | 0.212 | 0.194 |
| read_10 | 0.869 | 0.887 | -2.0% lower | 0.978 | 1.018 |
| edit_20 | 1.856 | 1.943 | -4.6% lower | 3.210 | 1.882 |
| search_10k | 67.922 | 49.002 | 27.9% lower | 50.655 | 50.677 |
| vsh_glob_10k | 71.975 | 54.002 | 25.0% lower | 57.927 | 57.591 |
| rename_subtree_100 | 56.976 | 55.962 | 1.8% lower | 57.833 | 66.967 |
| delete_subtree_100 | 47.952 | 46.048 | 4.0% lower | 47.085 | 51.947 |
| vsh_remove_subtree_100 | 48.012 | 45.082 | 6.1% lower | 47.988 | 50.073 |
| massive_delete_5k | 175.077 | 131.162 | 25.1% lower | 139.005 | 138.012 |

Sampled RSS (separate instrumented run):

| Phase | Root MiB | Summed tree MiB | Max processes |
|---|---:|---:|---:|
| baseline | 36.31 | 56.09 | 5 |
| final | 33.42 | 42.22 | 2 |

## Python release

| Case | Before p50 ms | Final p50 ms | Change | Final p95 ms | Repeat p50 ms |
|---|---:|---:|---:|---:|---:|
| delete_subtree_100 | 48.875 | 45.981 | 5.9% lower | 48.011 | 46.904 |
| edit_20 | 1.840 | 1.774 | 3.6% lower | 2.114 | 1.769 |
| massive_delete_5k | 182.400 | 130.868 | 28.3% lower | 139.118 | 133.015 |
| noop | 0.181 | 0.287 | -58.3% lower | 0.316 | 0.153 |
| read_10 | 0.861 | 1.024 | -18.9% lower | 1.347 | 0.886 |
| rename_subtree_100 | 58.011 | 55.977 | 3.5% lower | 58.835 | 55.922 |
| search_10k | 68.130 | 48.689 | 28.5% lower | 49.578 | 49.045 |
| vsh_glob_10k | 73.716 | 51.964 | 29.5% lower | 52.533 | 51.775 |
| vsh_remove_subtree_100 | 47.801 | 45.057 | 5.7% lower | 47.020 | 45.049 |

Sampled RSS (separate instrumented run):

| Phase | Root MiB | Summed tree MiB | Max processes |
|---|---:|---:|---:|
| baseline | 53.86 | 77.78 | 5 |
| final | 57.81 | 97.28 | 7 |

## Limitations

- Local sequential release runs, not randomized trials or a universal performance guarantee.
- Confirmation runs are separate observations, not selected replacements for the final run.
- Small calls and durable I/O show run-to-run variation; report regressions as well as gains.
- Summed sampled RSS double-counts shared pages and can miss short peaks; it is not PSS.
- No model calls or billing were measured; resource savings are not monetary savings.
- The initial baseline-python report used a debug extension and is excluded.
