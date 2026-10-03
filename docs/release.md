# Release process

The release workflow builds both language surfaces from one tag. Ordinary branch
builds and default manual dispatches do not publish packages. A separate, explicit
source-distribution recovery option is described below.

## Artifact matrix

`workflow_dispatch` with `publish_sdist_only=false` is the build-only rehearsal.
It produces and validates:

- CPython 3.11, 3.12, 3.13, and 3.14 wheels;
- manylinux 2.28 x86_64 and aarch64;
- macOS x86_64 and arm64;
- Windows x86_64;
- one installable sdist containing the complete locked Rust workspace needed to build
  the matching extension and worker;
- one metadata-only `vbash` mirror wheel and sdist exact-pinned to `vsh-python`;
- twelve crates.io archives, including `vsh-execution` and `vsh-bash`, explicitly
  excluding the non-published PyO3 build crate;
- a deterministic `SHA256SUMS` manifest.

Every wheel is installed into an empty environment and must complete a real native
preview and commit using its bundled exact-version worker. The aggregate validator
rejects a missing platform/Python tag, missing or non-executable worker, missing native
extension, incomplete sdist workspace, mirror package code payload, loose/mismatched
mirror dependency, corrupt archive, or wrong version
before a publish job is eligible.

## Publication authority

Normal full publication requires a pushed `v<project.version>` tag. Creating a GitHub
Release alone is not the trigger. The tag path uses these GitHub environments:

- `crates-io` with `CARGO_REGISTRY_TOKEN`;
- `pypi` configured as a PyPI trusted publisher for both `vsh-python` and `vbash`.

The workflow publishes crates in dependency order and waits for every exact version to
be visible through the crates.io API before publishing its dependents. `vsh-runtime`
precedes `vsh`, and `vsh` precedes the mirror crate `vbash`. PyPI publication
runs only after all twelve crates succeed; `vsh-python` is published before the empty
`vbash` mirror distribution. Python artifacts receive GitHub build provenance
and are uploaded with uv trusted publishing.

Before the first irreversible tag, recheck registry ownership/availability and review
the release environments. On 2026-10-03, all ten existing crate handles were owned
by `fswair`; `vsh-execution` and `vsh-bash` were available for first publication.
Registry state is not a permanent build-time assumption.

### Source-distribution recovery

Setting `publish_sdist_only=true` on a manual dispatch is an explicit publishing
operation, not a rehearsal. It rebuilds, validates, attests and publishes only the
`vsh-python` sdist through the `pypi` environment. Use it only for deliberate release
recovery at the intended revision; it does not publish wheels, crates or the mirror.

## Pinned build supply chain

Release actions use immutable commit SHAs. The selected releases are recorded beside
each `uses:` line in `.github/workflows/publish.yml`. The build tools are exact:

- Rust 1.95.0;
- uv 0.12.1;
- Maturin 1.15.0;
- the dependency versions in `Cargo.lock` and `uv.lock`.

Keep the full build-only matrix and main CI green before pushing an immutable release
tag. Registry publication uses the workflow's scoped token/trusted-publishing authority;
do not publish from an ad hoc local credential path.
