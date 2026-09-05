# Performance baseline

The current optimization evidence is the paired release Rust/PyO3 matrix captured on
2026-09-05, with before, final and confirmation runs in
`benchmarks/results/2026-09-05/`. It covers the Monty 0.0.22 integration in VSH 0.4.0.
The 2026-08-29 matrix remains historical rewrite evidence, not the current benchmark.

Read:

- [Performance overview and results](performance.md)
- [Full reproducibility record](rust-rewrite/PERFORMANCE.md)
- [Coverage and merge floors](rust-rewrite/COVERAGE.md)
- [Plan validation and remaining hosted gates](rust-rewrite/PLAN_VALIDATION.md)

Historical reports under `playground/reports/` remain engineering evidence but must not
be mixed with the current native/PyO3 matrix. In particular, compare only equivalent
workloads, release profiles, machines, worker identities, and sample protocols.
