# VSH dual-registry release

The release workflow builds both language surfaces from one tag without publishing
anything during an ordinary branch build or manual `workflow_dispatch` run.

## Artifact matrix

`workflow_dispatch` is the safe build-only rehearsal. It produces and validates:

- CPython 3.11, 3.12, 3.13, and 3.14 wheels;
- manylinux 2.28 x86_64 and aarch64;
- macOS x86_64 and arm64;
- Windows x86_64;
- one installable sdist containing the complete locked Rust workspace needed to build
  the matching extension and worker;
- eight crates.io archives, explicitly excluding the non-published PyO3 build crate;
- a deterministic `SHA256SUMS` manifest.

Every wheel is installed into an empty environment and must complete a real native
preview and commit using its bundled exact-version worker. The aggregate validator
rejects a missing platform/Python tag, missing or non-executable worker, missing native
extension, legacy Python engine path, incomplete sdist workspace, corrupt archive, or
wrong version before a publish job is eligible.

## Publication authority

Only a pushed `v<project.version>` tag enables publication. A manual workflow run does
not publish. The tag path requires protected GitHub environments:

- `crates-io` with `CARGO_REGISTRY_TOKEN`;
- `pypi` configured as a PyPI trusted publisher.

The workflow publishes crates in dependency order and waits for every exact version to
be visible through the crates.io API before publishing its dependents. PyPI publication
runs only after all eight crates succeed. Python artifacts receive GitHub build
provenance and are uploaded with uv trusted publishing.

Before the first irreversible tag, recheck registry ownership/availability and review
the release environments. The intended crate names were unclaimed when checked during
implementation, but that observation is not permanent.

## Pinned build supply chain

Release actions use immutable commit SHAs. The selected releases are recorded beside
each `uses:` line in `.github/workflows/publish.yml`. The build tools are exact:

- Rust 1.95.0;
- uv 0.12.1;
- Maturin 1.15.0;
- the dependency versions in `Cargo.lock` and `uv.lock`.

No registry publication was performed while implementing the rewrite.
