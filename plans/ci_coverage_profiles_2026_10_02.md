# Interrupted-worker coverage collection

## Design

User request: fix the failed main workflow, push the correction and verify hosted
CI. No release, dependency update, coverage-floor reduction or new exclusion.

Evidence: run 37028489320 passes all 296 Rust tests but fails llvm-profdata merge
on a corrupt raw-profile header. The first integration run had the same binary
signature; intervening runs passed. Workers intentionally have bounded shutdown
and forced termination. Exit-time profile writes are not reliable across this
boundary. The existing log does not identify the specific producer process.

Use LLVM continuous profiling for the Linux coverage job, with runtime counter
relocation and per-process/per-binary filenames. This collects counters before
exit, including forced termination. Do not change worker deadlines, cancellation,
security behavior or production compilation. Invalid profiles must still fail.

Primary references:
- https://clang.llvm.org/docs/SourceBasedCodeCoverage.html
- https://github.com/llvm/llvm-project/blob/release/22.x/compiler-rt/test/profile/ContinuousSyncMode/runtime-counter-relocation.c
- https://github.com/taiki-e/cargo-llvm-cov/blob/v0.9.0/README.md#environment-variables

## Implementation and acceptance

1. Add a small CI tooling regression that compiles an instrumented Rust fixture,
   waits for an execution marker, SIGKILLs it and verifies both a reached function
   count and an unreached function's zero count through llvm-cov. Use the selected
   rustc's matching LLVM tools; bound every subprocess and preserve failure.
2. Run that check under the same continuous-coverage flags used by the Linux
   coverage step. Keep all tests, exclusions and 79/70/81 floors unchanged.
3. Validate the tooling locally, inspect the diff, push main and follow hosted CI.
   A failed regression or profile merge invalidates the candidate. Record actual
   evidence; do not retry unchanged runs until green or filter corrupt profiles.

This is a collection-layer fix, not proof of the exact PID's shutdown race. The
kill regression must demonstrate the relevant failure boundary independently.

Local verification: Ruff lint/format, ty and basedpyright pass. A native macOS
probe correctly fails the nonzero-counter assertion: Darwin needs page-aligned
sections and uses a different mapping implementation. The script explicitly
targets Linux; neither that failed probe nor LLVM documentation alone is Linux
execution proof. Acceptance remains the hosted SIGKILL regression plus full CI.
