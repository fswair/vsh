# Performance baseline

The active VSH 0.3 baseline is the paired native Rust and PyO3 release matrix captured
on 2026-08-29. It replaces legacy Python command-engine comparisons as the merge
baseline for the rewrite.

Read:

- [Performance overview and results](performance.md)
- [Full reproducibility record](rust-rewrite/PERFORMANCE.md)
- [Coverage and merge floors](rust-rewrite/COVERAGE.md)
- [Plan validation and remaining hosted gates](rust-rewrite/PLAN_VALIDATION.md)

Historical reports under `playground/reports/` remain engineering evidence but must not
be mixed with the current native/PyO3 matrix. In particular, compare only equivalent
workloads, release profiles, machines, worker identities, and sample protocols.
