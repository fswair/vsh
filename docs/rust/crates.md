# Crate map

The workspace is split by authority and change rate. Applications normally depend only
on `vsh-runtime`; the other crates make boundaries auditable and publishable.

| Cargo package | Library/binary | Responsibility |
|---|---|---|
| `vsh-runtime` | `vsh` | Public native SDK and orchestration facade |
| `vsh-types` | `vsh_types` | Paths, digests, node state, transaction IDs and lifecycle |
| `vsh-vfs` | `vsh_vfs` | Immutable snapshots and copy-on-write virtual effects |
| `vsh-policy` | `vsh_policy` | Protected capabilities, deterministic decision and risk manifests |
| `vsh-monty` | `vsh_monty` | Typed Monty calls, limits, supervision, and result contract |
| `vsh-store` | `vsh_store` | Immutable blobs, approvals, and atomic transaction state |
| `vsh-commit` | `vsh_commit` | Revalidation, journaled mutation, verification, and recovery |
| `vsh-monty-worker` | binary | Exact-version crash-isolated guest execution process |
| `vsh-python` | `vsh._native` | Non-published PyO3 adapter used by the `vbash` wheel |

## Dependency direction

```text
vsh-types
  ├── vsh-vfs
  ├── vsh-policy
  └── vsh-store
       \
vsh-monty ── vsh-commit
       \       /
        vsh-runtime
             |
          vsh-python
```

The diagram is conceptual: consult the workspace manifests for the exact graph. The
important rule is authority flow—PyO3 adapts the runtime, and only the commit crate owns
host mutation.

## Which crate should I depend on?

- **Application or agent host:** `vsh-runtime`.
- **Custom receipt/state tooling:** prefer types re-exported by `vsh-runtime` before
  adding a direct lower-level dependency.
- **Alternative deterministic policy:** `vsh-policy` types are already re-exported.
- **Worker packaging:** ship the exact matching `vsh-monty-worker` binary; do not mix
  worker/runtime versions.
- **Python extension:** consume `vbash`; do not depend on `vsh-python` from crates.io
  because it is intentionally not published as a standalone crate.

## Version and supply-chain contract

All workspace packages share version `0.3.0`, Rust 1.95.0, edition 2024, and Apache-2.0.
Direct external crates are exact-pinned, the lockfile is committed, workspace crates
forbid unsafe code, and CI runs RustSec plus license/source/duplicate policy checks.

See the [dependency admission policy](../rust-rewrite/DEPENDENCY_POLICY.md) and
[release artifact order](../rust-rewrite/RELEASE.md).
