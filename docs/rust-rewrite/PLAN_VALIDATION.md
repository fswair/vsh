# Rust rewrite plan validation

Validated: 2026-09-02\
Scope: current working tree against `plans/vsh_rust_rewrite_dual_sdk.md`

## Requirement matrix

| Plan requirement | Result | Evidence |
|---|---|---|
| One semantic core for Rust and Python | Satisfied locally | `vsh-runtime` owns the implementation behind the public `vsh` facade; `vsh._native` is a thin PyO3 adapter. The shipped Python surface contains no simulator, policy, transaction state machine, or committer fallback. |
| Native crates.io SDK plus PyPI SDK | Satisfied locally | Ten publishable crate archives, the `vsh-python` native wheel/sdist, and metadata-only `vbash` compatibility artifacts build locally; the native wheel installs in an empty environment and performs a bundled-worker preview and real commit. |
| Exact, reviewed, active dependencies | Satisfied for the supported target graph | Direct Rust dependencies are exact-pinned, `Cargo.lock`/`uv.lock` are frozen, and a fresh downstream `vbash` graph compiles with the exact Monty-compatible `get-size2 =0.10.1` guard. 1,239 RustSec advisories were refreshed, `cargo-deny 0.20.2` passes advisories/bans/licenses/sources, and strict hash-locked `pip-audit 2.10.1` reports no vulnerability. The documented embedded-only `atomic-polyfill` lock entry is not selected by any supported target. |
| Capability-rooted complete mediation | Satisfied locally | Workspace, `.vsh-runtime`, data, blob, transaction, stage, quarantine, plan, journal, marker, and coordination handles are identity checked. Workspace/data overlap, canonical aliases, internal symlinks, root/runtime relocation, and parent swaps fail closed. |
| Zero host effects during simulation | Satisfied locally | Monty runs in a supervised exact-version worker with no host mount. Filesystem calls map to `VirtualFs`; protected and absolute host paths are denied before bytes enter the worker. |
| Bounded latency and memory surfaces | Satisfied for implemented paths | Program, OS calls, per-call/total I/O, events, frames, output, result, exception, paths, directory entries, snapshot nodes/depth/bytes, artifacts, state log, journal, plan, conflicts, and preview cache have hard bounds. Blob and host content verification stream or read through an explicit ceiling. |
| Deterministic durable state and single-use commit | Satisfied locally | Exact transaction binding, CAS lifecycle edges, reservation consumption, approval expiry, checksummed append log, two-slot compaction, durable intent/completion witnesses, post-commit verification, and recovery are exercised. Preflight failures after reservation end in `Failed`; complete checksum-corrupt frames never roll state backward. |
| Concurrency and crash recovery | Satisfied locally | Same-workspace commits serialize only revalidation/mutation; independent runtimes scale in parallel. Every injected durable commit boundary recovers to the original or committed state, and ambiguous ownership remains untouched. |
| Safe and low-overhead PyO3 boundary | Satisfied locally | Native calls release the GIL, including runtime open/recovery; panics are caught before crossing FFI; errors share `VshRuntimeError`; results use Monty's typed converter rather than JSON. The final 100-sample validation estimates 1.3–16.5 microseconds distinguishable incremental PyO3 p50. |
| Coverage is an enforced merge signal | Satisfied locally | 135 Rust tests measure 80.54% lines, 71.88% functions, and 82.55% regions; CI floors are 79/70/81. The 41 shipped-Python tests retain 100% line and branch coverage. See `COVERAGE.md` for boundary rationale. |
| Reproducible release workflow | Implemented; hosted execution outstanding | Versions agree at 0.3.1, actions/tools are immutable or exact-pinned, mirror packages are exact-version and payload-checked, local archives validate, and ordinary/manual workflows cannot publish. The 20-wheel hosted matrix, provenance, protected environments, registry visibility waits, and actual publication require GitHub/registry authority. |
| Dual-SDK documentation and agent integration | Satisfied locally | Zensical 0.0.57 builds 35 source pages in strict mode. Rust and Python APIs are separated; architecture, examples, use cases, benchmarks, security, MCP, and agent workflows are covered. Every page exposes its exact source through Copy as Markdown and supports automatic light/dark themes. `/llms.txt` indexes and `/llms-full.txt` concatenates all 35 Markdown sources. |

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

No new runtime dependency was introduced for these fixes, and workspace Rust crates continue to forbid `unsafe` code.

## Local gate record

- `cargo fmt --all -- --check`: pass.
- Fresh downstream `vbash` resolution and `use vsh::...` compile smoke: pass.
- `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings`: pass.
- `cargo llvm-cov --workspace --all-features --all-targets --locked ...`: 135/135 tests pass; 80.54/71.88/82.55 coverage.
- `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked --exclude vsh-runtime`: pass.
- PyO3/Python release-surface tests: 41/41 pass; 100% line and branch coverage.
- Ruff format/lint, ty, and basedpyright: pass.
- `cargo audit`: no known vulnerability or yanked package; one documented non-selected embedded-target maintenance warning.
- `cargo deny --all-features --locked check`: advisories, bans, licenses, and sources pass.
- Strict hash-locked Python audit: no known vulnerability.
- `zensical build --clean --strict`: 35/35 Markdown sources build; all 35 source-backed pages derive their exact Markdown URL and load the dark-red light/dark theme plus Copy as Markdown asset.
- `scripts/generate_llms_txt.py --check`: the compact index and 35-page full corpus are deterministic and current; both are copied unchanged to the site root.
- Documentation examples: all 20 Python code blocks pass syntax compilation; custom JavaScript passes `node --check`.
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
  the first irreversible publish.
