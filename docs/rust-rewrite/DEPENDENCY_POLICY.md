# Rust dependency policy and initial admission record

Checked: 2026-09-05

## Admission gate

Every direct crate requires:

1. a stable, non-yanked exact version,
2. active maintenance from an identifiable established project,
3. official crates.io/source metadata,
4. acceptable license,
5. compatible MSRV,
6. justified minimal features,
7. no known unwaived RustSec/CVE advisory in the resolved lockfile,
8. a reason `std` or an existing dependency is insufficient,
9. review of transitive and native-library additions,
10. a removal/replacement path.

Cargo requirements use `=x.y.z`; `Cargo.lock` is committed. Git dependencies,
wildcards, unknown registries, abandoned crates, and convenience-only dependencies are
not accepted.

## Initially admitted ecosystem

| Item | Pin | Authority | Purpose | Notes |
|---|---:|---|---|---|
| Rust | `1.95.0` | Rust project | compiler/toolchain | Matches Monty 0.0.22 MSRV. |
| `monty` | `=0.0.22` | Pydantic | interpreter | Stable published crate; MIT. |
| `monty-types` | `=0.0.22` | Pydantic | typed calls/limits | Same release train and MSRV. |
| `monty-proto` | `=0.0.22` | Pydantic | worker wire protocol and lossless Monty value → native Python conversion | Official same-release protocol/converter; worker feature disabled. |
| `monty-alloc` | `=0.0.22` | Pydantic | worker-wide bounded allocator | Official same-release allocator; only `exit-code`, defaults disabled. |
| `get-size2` | `=0.10.3` | get-size2 project | Monty/Ruff transitive memory accounting | Exact compatible release used by Monty 0.0.22's Ruff 0.0.9 and `compact_str 0.10` graph. |
| `pyo3` | `=0.29.2` | PyO3 | CPython binding | MIT/Apache-2.0; Rust 1.83 floor. |
| `blake3` | `=1.8.7` | BLAKE3 team | content/snapshot/diff identity | Official active implementation; only `std`, no rayon/mmap/serde. |
| `cap-std` | `=4.0.3` | Bytecode Alliance | capability-rooted host filesystem access | Stable/non-yanked latest; no default features; Apache-2.0 WITH LLVM-exception/Apache-2.0/MIT. |
| `cap-fs-ext` | `=4.0.3` | Bytecode Alliance | stable Windows by-handle identity for `cap-std` metadata | Official same-release extension; Windows-only edge; only `std`, defaults disabled; same accepted license family as `cap-std`. |
| `postcard` | `=1.1.3` | postcard project | exact Monty result in durable approval artifact | Stable/non-yanked latest; direct edge enables only `alloc`; MIT/Apache-2.0. |
| Maturin | `==1.15.0` | PyO3 | wheel build/publish | Build tool, not runtime dependency. |
| Setuptools | `==84.0.0` | Python Packaging Authority | metadata-only `vbash` compatibility wheel/sdist | Build-only; no OSV match for 84.0.0 when checked 2026-09-02. |
| cargo-audit | `=0.22.2` | RustSec ecosystem | advisory gate | Install/run with `--locked`. |
| cargo-deny | `=0.20.2` | Embark ecosystem | source/license/advisory gate | Install/run with `--locked`. |
| cargo-llvm-cov | `=0.9.0` | taiki-e | stable Rust line/function/region coverage gate | Active immutable upstream release; CI-only tool installed with its published lockfile. |
| pip-audit | `==2.10.1` | Python Packaging Authority | Python advisory gate | Ephemeral CI tool; audits the hash-locked export with strict failure. |
| Zensical | `==0.0.57` | Zensical project | versioned documentation site | MIT; Python 3.10+; development-only dependency. Release 0.0.57 was published 2026-08-21. |

Published metadata and advisory sources reported no known vulnerability for Maturin
1.15.0 and Monty 0.0.22 at check time. This is point-in-time evidence, not a
claim that any dependency is intrinsically safe and not a substitute for auditing the
resolved Cargo.lock on every build.

The core Python wheel has no pure-Python runtime dependency. The metadata-only
`vbash` compatibility distribution exact-pins `vsh-python` and uses current,
exact-pinned Setuptools only as its isolated build backend. The optional MCP adapter
pins the actively maintained stable `fastmcp==3.4.7`; it is not imported by `vsh` or
the native SDK path. Maturin remains exact-pinned at `1.15.0` in both the build system
and developer environment. Zensical is exact-pinned at `0.0.57` only in the developer
and documentation graph, so it does not enlarge the wheel's runtime surface. Its
official release and PyPI metadata were current at review time, and the hash-locked
audit found no known vulnerability in its resolved graph. Python resolution is
committed in `uv.lock`. After the
2026-08-29 advisory refresh, the affected optional/development transitives were moved
to `cryptography==50.0.1`, `mcp==1.29.1`, `pydantic-settings==2.15.0`, and
`starlette==1.6.0`; all four are hash-pinned by that lockfile.

`cap-std` is admitted because ordinary path-based `std::fs` operations cannot preserve
the directory capability across rename races without a platform-specific descriptor
layer. VSH uses only its filesystem handles, disables defaults, and pins opened parent
directories through commit. Its upstream MSRV is below VSH's Rust 1.95 floor. Removal
requires an equally reviewed capability implementation and the same race suite.

`cap-fs-ext` is the Bytecode Alliance's official extension crate for the same
`cap-std` release. VSH selects it only on Windows, where Rust 1.95 does not expose
stable by-handle volume and file-index methods on `std::fs::Metadata`. Its `dev` and
`ino` accessors read the full identity already captured by `cap-std` handles; they do
not introduce ambient path authority. The edge can be removed once stable Rust or
`cap-std` exposes equivalent Windows identity without an extension trait.

`postcard` is admitted only for Monty's exact `MontyObject`, whose serde implementation
already made postcard part of the Monty 0.0.22 resolved graph. The direct VSH edge adds
no registry package and enables only `alloc`; VSH's surrounding artifact envelope is a
manual, bounds-checked, versioned codec. This keeps path/diff/policy data out of serde
and permits replacing postcard if Monty's public value representation changes.

`monty-proto` is admitted for two narrow uses. The worker and parent use its official,
typed protobuf messages and bounded frame helpers; `vsh-python` enables its official
`python` converter to map `MontyObject` directly to native CPython objects under the
GIL. This avoids JSON, `repr` parsing, and duplicate semantic mappings. The crate's
`worker` feature is intentionally disabled. Removal is straightforward if Monty moves
the converter/protocol into `monty-types` or exposes equivalent public APIs.

`monty-alloc` is admitted only in the isolated worker process. Its global limited
allocator makes the memory ceiling independent from Monty's bytecode-duration and VSH
I/O budgets. The parent process does not install it. The direct edge adds no unrelated
pool, async, telemetry, or networking dependency.

The published `monty-runtime =0.0.22` binary and `monty-proto/worker` feature were
evaluated and rejected. Their locked graph selects yanked `chacha20 =0.10.0` through
`monty-type-checking` and Ruff's notebook/random stack; a fresh resolution is also
unsatisfiable with the currently published constraints. VSH does not waive the yank,
patch crates.io, fork Monty, or copy that dependency graph. `vsh-monty-worker` is a
minimal same-workspace binary built on Monty's public `MontyRun`/`RunProgress` API,
the official protocol types, `monty-alloc`, and `std` supervision.

## Feature policy

- PyO3 starts with only the features needed for an extension module.
- Per-Python-version wheels are the default; `abi3-py311` is benchmark-gated.
- Monty's runtime/pool/type-checking/telemetry features are not selected.
- Core VSH crates do not pull async, TLS, HTTP, tracing exporters, or serialization
  frameworks unless the owning phase proves the need.
- Dev/benchmark dependencies receive the same advisory/source/license scrutiny.
- Monty's published `postcard` declaration enables its default `heapless-cas`
  feature. On the supported macOS/Linux/Windows targets this does not select
  `atomic-polyfill`; target-aware checks are mandatory so an unused universal-lock
  entry is not confused with shipped code.

## Required commands once Cargo.lock exists

```bash
cargo test --workspace --all-targets --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo doc --workspace --all-features --no-deps --locked --exclude vsh-runtime
cargo audit
cargo deny --all-features --locked check
cargo tree --duplicates --locked
uv export --quiet --frozen --all-groups --extra mcp --no-emit-project \
  --output-file /tmp/vsh-audit-requirements.txt
uvx --from pip-audit==2.10.1 pip-audit \
  --requirement /tmp/vsh-audit-requirements.txt --require-hashes \
  --disable-pip --strict --progress-spinner off
```

CI tool installation itself is exact and locked:

```bash
cargo install cargo-audit --version 0.22.2 --locked
cargo install cargo-deny --version 0.20.2 --locked
uvx --from pip-audit==2.10.1 pip-audit --version
```

## Current resolved graph result

The current lockfile contains eleven workspace packages and 227 registry package
entries. The direct external runtime dependencies are `monty =0.0.22`,
`monty-types =0.0.22`, `monty-proto =0.0.22`, `monty-alloc =0.0.22`, `pyo3 =0.29.2`,
`blake3 =1.8.7`, `cap-std =4.0.3`, `cap-fs-ext =4.0.3` (Windows only),
`postcard =1.1.3`, and the `get-size2 =0.10.3` resolution guard; the rest are exact
lockfile resolutions of those dependencies. The guard is carried by `vsh-monty` so
fresh downstream resolution remains on the reviewed Monty 0.0.22/Ruff 0.0.9 graph.
The newly selected transitive package names are `char_str =0.0.2` from Astral's Ruff
graph, `drop_bomb =0.1.5` from matklad through the Ruff parser, and `zmij =1.0.23`
from David Tolnay through `compact_str`. They are registry/checksum locked, have
identified upstream repositories, are not direct VSH dependencies, and are included in
the same advisory, source, license, and target scans.
The earlier Monty 0.0.21 incompatibility between `get-size2 0.10.3`/`compact_str 0.10`
and Ruff 0.0.3/`compact_str 0.9` no longer exists.
Checks on 2026-09-05 refreshed 1,239 RustSec advisories and scanned all 238 lockfile
packages after worker admission. `cargo audit` found no known vulnerability or yanked
package. In particular, neither `chacha20` nor `monty-type-checking` is present. Exact
registry metadata was refreshed for every new direct pin.
The same lockfile passed cargo-deny 0.20.2 with all workspace features across the
configured supported-target matrix: advisories, bans, licenses, and sources all pass.
The hash-locked Python export, including the optional MCP, development, and Zensical
documentation graphs, passed pip-audit 2.10.1 in strict mode on 2026-09-05 with no
known vulnerabilities.

The supported target matrix (`aarch64`/`x86_64` macOS, `aarch64`/`x86_64` Linux, and
`x86_64` Windows) reports:

- no known RustSec vulnerability or unsoundness advisory,
- no selected unmaintained crate,
- no yanked package,
- only crates.io/workspace sources,
- only reviewed licenses,
- no unrecorded duplicate-version exception.

There are ten exact, reviewed duplicate-version exceptions. Four come from Monty
0.0.22 and its Ruff/parser/proc-macro graph (`getrandom 0.2.17`, `hashbrown 0.16.1`,
`itertools 0.14.0`, and `syn 2.0.119`); six are platform-specific generations retained
by `cap-std 4.0.3` (`io-lifetimes 2.0.4`, `windows-sys 0.59.0`/`0.60.2`,
`windows-targets 0.52.6`, and its GNU/MSVC x86_64 artifacts). Exact inclusion reasons
are recorded inline in `deny.toml`; every new duplicate remains denied.

`cargo audit` additionally reports `RUSTSEC-2023-0089` for `atomic-polyfill 1.0.3` as
an unmaintained, lock-only package. The path is Monty's published default
`postcard -> heapless-cas`; `atomic-polyfill` is target-specific to embedded
architectures outside VSH's support matrix and `cargo tree` selects it for none of the
shipped targets. It has no vulnerability/CVE advisory, is not ignored in
`deny.toml`, and target-aware `cargo deny` passes. This upstream manifest issue is a
release-tracked exception: VSH must not add an embedded target or claim a warning-free
universal lock until Monty removes the default feature.

`deny.toml` still rejects unmaintained and unsound advisories across every selected
target graph, plus yanked packages, wildcard requirements, unknown registries, and
unknown Git sources. A clean scan means "no applicable advisory published in the
refreshed database at scan time"; it never replaces code review, provenance review, or
continuous rescanning.

## Pending admissions

Database backends, property-testing helpers, temporary-file support, external
worker-pool dependencies, and benchmarking libraries are not pre-approved by category.
The current worker pool is a bounded short-lock `std` implementation. Select future
candidates only when measurements establish a need, compare `std`/existing options,
verify current maintenance/advisories, then add an admission entry before changing
Cargo.toml.

## VSH registry packages

The same-workspace VSH packages are project-owned code, not admitted third-party
dependencies. On 2026-09-02 the crates.io owner API identified `fswair` as owner of
the transferred `vsh` and `vbash` handles as well as the already published `vsh-*`
graph. Each manifest permits only `crates-io` publication, and every inter-package
requirement is the exact lockstep version `=0.4.0`.

The intended first-publish order is:

```text
vsh-types → vsh-store → vsh-vfs → vsh-policy
          → vsh-commit + vsh-monty → vsh-runtime → vsh → vbash
                                                └→ vsh-monty-worker
```

`cargo package --workspace --exclude vsh-python --offline --no-verify --locked` produces
all ten registry source archives. The explicit exclusion matters because Cargo 1.95
also packages a `publish = false` workspace member when a whole workspace is selected.
Cargo 1.95.0 verifies the dependency-free `vsh-types` archive, then its temporary
multi-package registry currently stops with Cargo's internal `no hash listed for
vsh-types 0.4.0` error. Dependent dry-runs therefore become executable only in the
normal publish order after each predecessor exists in crates.io; this is not waived and
does not authorize publishing without an explicit release action.

## CI and release build supply chain

The hosted workflows do not use moving action tags. They pin the complete commit SHA
for checkout 7.0.1, setup-uv 9.0.0, Maturin action 1.51.0, artifact upload 7.0.1,
artifact download 8.0.1, and build-provenance attestation 4.2.2. Human-readable release
tags remain comments beside each SHA for auditability. setup-uv installs exact uv
0.12.1; the project and release action install exact Maturin 1.15.0 and Rust 1.95.0.

The release job builds wheels before it receives any publish authority, installs and
exercises every wheel, verifies the sdist can rebuild the extension plus separate
worker from the locked workspace, rejects legacy Python engine paths, validates all
eight crate archives, and emits SHA-256 hashes. Manual dispatch is build-only. Only an
exact version tag can enter protected crates.io and PyPI environments.
