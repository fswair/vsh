# VSH driver-process peak RSS

Captured: 2026-08-29 on macOS 26.1 arm64\
Measurement: `/usr/bin/time -l`\
Workload: seven-case expanded matrix, 30 warm samples per case, 10 independent cold
samples, and four parallel runtimes.

| Surface | maximum resident set | peak memory footprint | wall time |
|---|---:|---:|---:|
| native Rust | 26,705,920 bytes (25.47 MiB) | 21,693,440 bytes | 11.91 s |
| CPython 3.14 + PyO3 | 48,021,504 bytes (45.80 MiB) | 36,013,376 bytes | 12.38 s |

The values are driver-process high-water marks. They do not sum the separately
supervised Monty worker processes, so they are suitable for native-vs-binding driver
comparison but not a whole-process-tree memory ceiling. Cross-platform and worker-tree
RSS remain CI/release measurements.
