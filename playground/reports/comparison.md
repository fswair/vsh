# Benchmark comparison: baseline vs optimized

Both runs: **50 iterations**, `file_count=20`, `file_size=256`, modes `native,vsh_apply,vsh_full`.

| | baseline | optimized |
|--|----------|-----------|
| commit | `42c29805ff60c165fb33847e588497b9eb56e909` | working tree (perf opts) |
| report | `playground/reports/baseline/` | `playground/reports/optimized/` |

## vsh_full median (ms)

| command | baseline | optimized | delta | speedup |
|---------|---------:|----------:|------:|--------:|
| cat | 0.763 | 0.741 | -0.022 | 1.03x |
| cd | 0.732 | 0.751 | +0.019 | 0.98x |
| chmod | 0.779 | 0.904 | +0.126 | 0.86x |
| cp | 1.148 | 1.099 | -0.048 | 1.04x |
| du | 1.588 | 1.266 | -0.322 | 1.25x |
| echo | 0.601 | 0.658 | +0.057 | 0.91x |
| echo_write | 0.852 | 0.940 | +0.088 | 0.91x |
| find | 1.883 | 1.675 | -0.208 | 1.12x |
| grep | 2.484 | 2.088 | -0.395 | 1.19x |
| head | 0.833 | 0.804 | -0.029 | 1.04x |
| ln | 0.966 | 0.868 | -0.098 | 1.11x |
| ls | 1.320 | 1.024 | -0.296 | 1.29x |
| mkdir | 0.931 | 0.860 | -0.072 | 1.08x |
| mv | 1.037 | 0.889 | -0.148 | 1.17x |
| nl | 0.876 | 0.751 | -0.125 | 1.17x |
| pwd | 0.712 | 0.639 | -0.073 | 1.11x |
| rg | 2.519 | 2.018 | -0.501 | 1.25x |
| rm | 0.871 | 0.802 | -0.069 | 1.09x |
| sed | 0.833 | 0.720 | -0.113 | 1.16x |
| sed_inplace | 0.913 | 0.785 | -0.129 | 1.16x |
| sort | 0.843 | 0.727 | -0.116 | 1.16x |
| stat | 0.807 | 0.752 | -0.055 | 1.07x |
| tail | 0.884 | 0.833 | -0.051 | 1.06x |
| touch | 0.898 | 0.935 | +0.037 | 0.96x |
| wc | 0.877 | 0.758 | -0.119 | 1.16x |

**Summary:** 20/25 commands faster on optimized; total vsh_full median sum 26.9ms → 24.3ms (**1.11x**).

## Largest improvements (optimized faster)

- **ls**: 1.320ms → 1.024ms (1.29x)
- **du**: 1.588ms → 1.266ms (1.25x)
- **rg**: 2.519ms → 2.018ms (1.25x)
- **grep**: 2.484ms → 2.088ms (1.19x)
- **mv**: 1.037ms → 0.889ms (1.17x)
- **nl**: 0.876ms → 0.751ms (1.17x)
- **sed_inplace**: 0.913ms → 0.785ms (1.16x)
- **sort**: 0.843ms → 0.727ms (1.16x)
- **wc**: 0.877ms → 0.758ms (1.16x)
- **sed**: 0.833ms → 0.720ms (1.16x)

## Regressions (optimized slower)

- **chmod**: 0.779ms → 0.904ms (0.86x)
