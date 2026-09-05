# Rust rewrite plan validation

Validated: 2026-09-05\
Scope: current working tree against `plans/vsh_rust_rewrite_dual_sdk.md`

## Requirement matrix

| Plan requirement | Result | Evidence |
|---|---|---|
| One semantic core for Rust and Python | Satisfied locally | `vsh-runtime` owns the implementation behind the public `vsh` facade; `vsh._native` is a thin PyO3 adapter. The shipped Python surface contains no simulator, policy, transaction state machine, or committer fallback. |
| Native crates.io SDK plus PyPI SDK | Satisfied locally | Ten publishable crate archives, the `vsh-python` native wheel/sdist, and metadata-only `vbash` compatibility artifacts build locally; the native wheel installs in an empty environment and performs a bundled-worker preview and real commit. |
| Exact, reviewed, active dependencies | Satisfied for the supported target graph | Direct Rust dependencies are exact-pinned, `Cargo.lock`/`uv.lock` are frozen, and Monty, its companion crates, and `get-size2` are pinned to `=0.0.22` and `=0.10.3`. 1,239 RustSec advisories were refreshed, `cargo-deny 0.20.2` passes advisories/bans/licenses/sources, and strict hash-locked `pip-audit 2.10.1` reports no vulnerability. The documented embedded-only `atomic-polyfill` lock entry is not selected by any supported target. |
| Capability-rooted complete mediation | Satisfied locally | Workspace, `.vsh-runtime`, data, blob, transaction, stage, quarantine, plan, journal, marker, and coordination handles are identity checked. Workspace/data overlap, canonical aliases, internal symlinks, root/runtime relocation, and parent swaps fail closed. |
| No guest writes to host workspace files during simulation | Satisfied locally | Monty 0.0.22 runs in a supervised exact-version worker with no host mount. Typed filesystem calls and the ten injected `vsh_*` functions operate on the same caller-owned `VirtualFs`; protected and absolute host paths are denied before bytes enter the worker. The host runtime still writes its own storage and transaction artifacts. |
| Bounded latency and memory surfaces | Satisfied for implemented paths | Program, typed OS/high-level call count, per-call/total I/O, discovery results, events, frames, output, result, exception, paths, directory entries, snapshot nodes/depth/bytes, artifacts, state log, journal, plan, conflicts, and preview cache have hard bounds. Glob/search traverse incrementally and stop at their requested result limit; blob and host content verification stream or read through an explicit ceiling. |
| Deterministic durable state and single-use commit | Satisfied locally | Exact transaction binding, CAS lifecycle edges, reservation consumption, approval expiry, checksummed append log, two-slot compaction, durable intent/completion witnesses, post-commit verification, and recovery are exercised. Preflight failures after reservation end in `Failed`; complete checksum-corrupt frames never roll state backward. |
| Concurrency and crash recovery | Satisfied locally | Same-workspace commits serialize only revalidation/mutation; independent runtimes scale in parallel. Every injected durable commit boundary recovers to the original or committed state, and ambiguous ownership remains untouched. |
| Safe and low-overhead PyO3 boundary | Satisfied locally | Native calls release the GIL, including runtime open/recovery; panics are caught before crossing FFI; errors share `VshRuntimeError`; results use Monty's typed converter rather than JSON. The earlier paired 100-sample validation estimates 1.3–16.5 microseconds distinguishable incremental PyO3 p50; it is not the current optimization benchmark. |
| Coverage is an enforced merge signal | Satisfied locally | 154 Rust tests measure 81.14% lines, 73.43% functions, and 83.01% regions; CI floors are 79/70/81. The 48 shipped-Python tests retain 100% line and branch coverage. See `COVERAGE.md` for boundary rationale. |
| Reproducible release workflow | Implemented; hosted execution outstanding | Versions agree at 0.4.0, actions/tools are immutable or exact-pinned, mirror packages are exact-version and payload-checked, local archives validate, and ordinary/manual workflows cannot publish. The 20-wheel hosted matrix, provenance, protected environments, registry visibility waits, and actual publication require GitHub/registry authority. |
| Dual-SDK documentation and agent integration | Satisfied locally | Zensical 0.0.57 builds 39 source pages in strict mode. Separate SDK references, fixture-owning cookbooks, lifecycle/efficiency guidance, current benchmark evidence and VSH 0.4.0 availability are documented. Every source page exposes Copy as Markdown and light/dark themes; the LLM bundles include all 39 sources. |

The package/security validation entries above include earlier release evidence. They
are not a claim that registry publication, vulnerability databases or every archive
were revalidated during the optimization pass below.

## Findings resolved during validation

1. `Runtime.open` held the GIL while Rust opened/recovered stores. It now detaches around the native operation and has a deterministic lock-contention thread test.
2. Ambient `.vsh-runtime/data` and blob paths could be redirected by symlinks. Storage now uses pinned `DataDirectory` capabilities and real-child identity checks.
3. Explicit data directories could overlap the workspace through lexical, canonical, or prospective aliases. Preflight and post-open separation checks reject them before persistent writes.
4. A renamed/replaced workspace or protected runtime directory could leave a valid old handle. The committer records and revalidates both identities before and after observation/mutation boundaries.
5. Commit preflight failures after consuming a reservation could leave `Reserved` stranded. Binding, size, lock, and identity failures now atomically finalize it as `Failed`.
6. A full final state-log frame with a bad checksum was treated like a torn append and silently truncated. Only incomplete EOF data is repaired now; a complete corrupt frame fails closed.
7. Blob verification materialized the entire file. It now hashes in fixed-size chunks, and bounded retrieval rejects oversized content before returning bytes.
8. Lazy host reads, journal/marker/plan recovery reads, and directory revalidation retained growth races that could allocate beyond the pre-observed size. Reads now use explicit `limit + 1` ceilings, and directory revalidation has an entry bound.
9. Recovery could follow a replaced internal plan/journal/marker symlink. Existing internal files and recovery transaction directories are now opened only after no-follow type and identity validation.
10. Python internal failures inherited directly from `RuntimeError`, bypassing the public VSH base. `VshInternalError` now derives from `VshRuntimeError`, with matching stubs and tests.
11. Public Rust error/value contracts and worker frame/protobuf edge cases lacked direct coverage. Stable message/source and bounded protocol tests now cover them.
12. Subprocess coverage left `*.profraw` files beside the worker sources, so Cargo included 77 test artefacts in the worker crate. They were removed, globally ignored, and the final worker archive dropped from 85 files to the intended 8.
13. Monty 0.0.22 changed instance storage, function-call identity, suspension limits, and
    protocol variants. The adapter and worker now use the public 0.0.22 contracts and
    reject unknown external calls while injecting only the ten typed VSH functions.
14. The first high-level discovery implementation materialized complete trees and used
    recursive glob matching. It now streams deterministic traversal, stops at
    `max_results`, and uses an iterative matcher that remains bounded for deep `**`
    patterns.
15. Case-fold expansion could produce a byte offset that was invalid in the original
    UTF-8 line, and worker framing treated function calls as small control messages.
    Unicode search now maps folded matches back to original character columns, while
    function-call payloads use the typed-call frame limit.
16. Policy/path matching repeatedly allocated component vectors and DP rows. The new
    constant-space matcher, canonical path fast path and borrowed lookups preserve
    authorization order and normalization, verified against independent oracles.
17. Snapshot indexing repeated parent work, and overlay listings scanned unrelated
    paths. Fused construction, empty-overlay shortcuts and slash-bounded ranges reduce
    work without stale snapshot reuse or a second permanent index.
18. Canonical diff's temporary keyed after-state tree is now a flat buffer, retaining
    complete lazy after-materialization before any before-state reads.
19. Documentation overstated preview isolation, output limits and CLI artifact lifetime.
    Current guides and executed SDK/MCP/separate-process CLI examples now make those
    boundaries explicit. APFS's pre-capture rejection of invalid UTF-8 fixture names
    is also handled explicitly in the platform test.

No new runtime dependency was introduced for these fixes, and workspace Rust crates continue to forbid `unsafe` code.

## Current optimization and documentation gate record

- `cargo fmt --all -- --check`: pass.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: pass.
- `cargo test --workspace --all-targets --all-features --locked`: 154/154 tests pass.
- `cargo llvm-cov --workspace --all-features --all-targets --locked ...`: 154/154 tests pass; 81.14/73.43/83.01 line/function/region coverage.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked --exclude vsh-runtime`: pass.
- PyO3/Python release-surface tests: 48/48 pass; 100% line and branch coverage.
- Ruff format/lint, CI-scoped ty, and basedpyright: pass. Historical legacy tests are
  not the current wheel's type-check contract.
- `zensical build --clean --strict`: 39 Markdown source pages build.
- `scripts/generate_llms_txt.py --check`: source corpus and LLM bundles are deterministic.
- `scripts/check_docs.py`: 39 source pages, 3,214 local references and 23 Python
  snippets checked; no broken links, fragments, source mappings or syntax errors.
  The docs-only workflow enforces this without compiling Rust. The 404 page now
  provides a working keyboard skip target.
- Browser checks: 39 pages at 320px light and 1440px dark; no horizontal page overflow,
  toolbar/title overlap, missing copy mappings or duplicate copy buttons. All 88
  representative interaction/theme contrast checks pass.
- Browser interactions: search, Python/Rust tabs, persistent theme switching,
  instant-navigation source copying and the mobile drawer pass without page errors.
- Native Rust and Python release baseline/final/confirmation matrices: 40 warm samples,
  20 cold samples, four independent runtimes; raw reports under
  `benchmarks/results/2026-09-05/`. Large-workload latency improvements repeat; RSS
  improvement is not established. See [performance](../performance.md) for caveats.
- Python/MCP/CLI fixture recipes and the first-run documentation block execute in the
  Python suite. The Rust staged-release recipe also executes and its shared guest
  source is present in the runtime crate's package file list.

## Earlier release validation retained for provenance

These checks predate the optimization/doc changes and were not rerun as fresh release
certification in this pass:

- Fresh downstream `vbash` resolution and `use vsh::...` compile smoke: pass.
- `cargo audit`: no known vulnerability or yanked package; one documented non-selected embedded-target maintenance warning.
- `cargo deny --all-features --locked check`: advisories, bans, licenses, and sources pass.
- Strict hash-locked Python audit: no known vulnerability.
- Local CPython 3.14 wheel, sdist, metadata-only compatibility pair, and ten crate archives: structure validation passes; isolated wheel preview/commit smoke passes.
- Paired 100-sample native/PyO3 benchmark plus 30 cold starts: binding overhead and independent-runtime scaling budgets pass; raw JSON and Markdown are retained under `playground/reports/rust-rewrite-python-v0.3.0/`.

## Remaining gates

There is no known code-side blocker in the locally exercised macOS arm64/Python 3.14
surface. These are release gates, not locally solvable implementation defects:

- run the hosted Linux/macOS/Windows and CPython 3.11–3.14 wheel matrix;
- compare the complete matrix with a clean, frozen legacy-Python baseline;
- capture supported-platform whole worker-tree RSS and adversarial performance cases;
- freeze the baseline/release identities in clean commits and tags;
- verify registry ownership, protected environments, credentials, and provenance before
  the next irreversible publish.
