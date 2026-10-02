# Shared filesystem gateway — implementation evidence

Updated 2026-10-01. Baseline: `8550facbe94485cb12bfe007fbb3575a168b6afc`.
Local changes are uncommitted; no push, tag or release has been performed.

## Delivered

- Added `crates/vsh-execution`: borrowed, policy-aware filesystem gateway, neutral path mapping, work budgets/stats/errors, and explicit effect origin. It has no external direct dependencies, guest interpreter, host filesystem/process calls, snapshot clone or global mutex.
- Routed Monty OS calls and high-level filesystem tools through it. Object/exception conversion, Python handle semantics, text search, glob parsing and tool argument handling remain Monty concerns. Existing imports of limits/stats/path types are preserved through re-exports.
- Moved tree copy/open preparation/visible traversal into the authority layer. Read-only observed entries let search use the already observed size without a duplicate metadata read or a raw mutable filesystem reference. Early traversal stop still avoids visiting descendants.
- Fixed directory-rename child authorization: preflight every source and rebased destination, including protected paths and rebased path limits, before mutation. This deliberately adds traversal evidence/work.
- Bound directory enumeration before full-list allocation for listings, visible walks, copy, remove and rename. Ordered base/overlay iterators merge without a second full candidate BTreeSet; over-limit reads stop at the first excess entry and cannot return a successful partial list. Hidden names consume work before filtering; rejected listings retain a metadata dependency but emit no successful directory-read effect.
- Made budget addition overflow fail closed even at a `u64::MAX` cap. Failed work does not refund already accepted counters.
- Bound the stronger semantics into Monty config identity: `vsh-monty-config-v4` includes `vsh-fs-gateway-v1`. Pending handles must not silently inherit changed semantics.
- Added `vsh-execution` to workspace version checks, source-artifact validation and topological publishing order. Version remains the existing workspace 0.5.0; this is not another 0.5.0 publication.

## Verification

All checks below are local macOS/arm64 evidence, not a hosted CI or cross-platform claim.

| Check | Result |
| --- | --- |
| Workspace/all-features/all-targets Rust tests | 196 passed; includes 30 new shared-boundary tests |
| Rebuilt release Python binding and six release-surface suites | 153 passed; 100% line and branch coverage |
| Isolated Bashkit upstream contract probe | 14 passed; exact 0.18.2, default features disabled |
| Rust line/function/region coverage | 81.80% / 75.47% / 83.34%; existing floors 79 / 70 / 81 pass |
| Shared open-preparation line coverage | 97.37%; missing reads, directory/link targets, bad parents, truncation, append/create, modes and deferred content accounting exercised |
| Workspace/all-features Clippy | Passed with `-D warnings` |
| Workspace rustdoc | Passed with `-D warnings`, existing runtime exclusion retained |
| Workspace + isolated probe formatting; tracked diff whitespace | Passed |
| Workspace version check | Passed |
| Source archive packaging + validation | All 11 crate archives passed; packaging used `--no-verify`, not a registry consumer compile |
| Monty-only `cargo tree -p vsh -e features` | No Bashkit or Tokio entries |

Cached `cargo audit --no-fetch --no-yanked` found no vulnerability advisory, but reported the existing **RUSTSEC-2023-0089 maintenance warning** for `atomic-polyfill` in the all-target lock graph (`postcard -> heapless`). It is not present in the current arm64 host tree and is not a newly added dependency. Do not describe this invocation as a fresh yanked-version check or a warning-free audit. `cargo deny --offline --all-features --locked check` passed advisories/bans/licenses/sources; its optional standard-library replacement-data cache emitted a diagnostic. No dependency exceptions or coverage exclusions were added.

### Reproduction

```sh
env CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo test --workspace --all-features --all-targets --locked
env CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo clippy --workspace --all-features --all-targets --locked -- -D warnings
env CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 cargo llvm-cov --workspace --all-features --all-targets --locked --summary-only --ignore-filename-regex '(vsh-python|vsh-worker)' --fail-under-lines 79 --fail-under-functions 70 --fail-under-regions 81
env RUSTDOCFLAGS='-D warnings' cargo doc --workspace --all-features --no-deps --locked --exclude vsh-runtime
env VIRTUAL_ENV="$PWD/.venv" .venv/bin/maturin develop --release --skip-install --offline
env VSH_MONTY_WORKER="$PWD/target/release/vsh-monty-worker" .venv/bin/python -m pytest tests/test_main.py tests/test_native_binding.py tests/test_native_runtime.py tests/test_commit_judge.py tests/test_pydantic_ai_capability.py tests/test_python_surface.py --cov=src/vsh --cov-branch --cov-report=term-missing --cov-fail-under=100
python3 release/check_versions.py
cargo package --workspace --exclude vsh-python --offline --no-verify --locked --allow-dirty
python3 release/validate_artifacts.py target/package --version 0.5.0 --wheel-count 0 --crate-count 11 --python-surface none
```

The editable native build does not install a bundled worker. Build `cargo build -p vsh-monty-worker --release --locked` first and supply its absolute path for local tests. A missing-worker run failed before this path was configured; the configured suites passed. Disk-pressure builds also failed before task-owned debug caches were cleared; those were not test/assertion failures. Only regeneratable build caches were removed; benchmark evidence and source were retained.

## Performance evidence — gate not closed

Used the existing `vsh-runtime` `native_benchmark` release example, the same explicit release worker, 100 warm / 5 cold iterations for the final runs, and isolated benchmark workspaces. Archived the baseline checkout separately and saved its executable before rebuilding the candidate. Do not build two different local source checkouts into one shared Cargo target: that produced an invalid artifact mix during setup and was corrected before measurements.

Generated records remain under ignored `target/bashkit-evidence/p2/`; durable results below are milliseconds. Baseline final record: `rust-baseline-final.json`; current gateway candidate: `rust-candidate-bounded-trees.json`.

| Case | Baseline p50 | Candidate p50 | Baseline p95 | Candidate p95 |
| --- | ---: | ---: | ---: | ---: |
| No-op | 0.217 | 0.298 | 0.272 | 0.425 |
| Read 10 | 0.928 | 0.933 | 1.295 | 1.324 |
| Edit 20 | 1.885 | 1.891 | 3.073 | 3.022 |
| Search 10k | 62.623 | 61.904 | 64.951 | 62.728 |
| Glob 10k | 67.064 | 66.533 | 68.447 | 68.184 |
| Rename subtree 100 | 69.904 | 69.323 | 76.087 | 70.423 |
| Delete subtree 100 | 58.989 | 58.103 | 60.097 | 59.972 |
| Tool remove subtree 100 | 71.406 | 57.937 | 81.128 | 61.042 |
| Delete 5k | 152.496 | 149.385 | 159.216 | 176.609 |

Interpretation, not marketing claims:

- Most nontrivial p50 values are close; this does **not** prove a speedup. The unchanged baseline's own repeats varied materially: no-op p50 ranged approximately 0.217–0.376 ms, and tool-remove p50 ranged 59.747–71.406 ms. High background CPU load was observed during the earlier noisy run.
- Candidate no-op/tail results still miss the planned parity criterion. Do not dismiss them or mark the performance gate green. A quiet paired/interleaved control run and no-op stage investigation remain required.
- Rename execution itself changed from about 0.266 ms to 0.504 ms for 100 children, despite similar total wall time. Roughly 0.24 ms is the measured cost of the additional child authorization/observation work. Snapshot timing must not hide this cost.
- Found and removed duplicate traversal policy checks and eager child filtering; restored lazy early-stop behavior. The bounded directory merge also removes a full candidate-set allocation, but no process-memory/RSS improvement has yet been measured.

## Remaining work and boundary

This is the shared-authority preparation, **not a usable Bash backend**. There is no production Bash dependency, language switch, Bash worker/RPC, chmod mutation primitive, mode effect/artifact encoding, Bash receipt, Bash Python/capability/MCP surface, or release documentation.

Before attaching Bash: close the P2 performance gate and independently review gateway semantics. Resolve nested AST/profile enforcement from P0: `before_exec` is outer-only; `before_tool` is not an all-source parser guard. Then implement P3 metadata-only mode mutation with policy, canonical diff, stale/commit/journal recovery tests; P4 supervised worker transport and hard resource handling; and P5–P9 runtime/evidence/bindings/security/docs. Fresh combined Bash dependency audit/deny, Linux/macOS conformance, malformed protocol/cancellation/reset tests and worker-memory measurements are still required.
