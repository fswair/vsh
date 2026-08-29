# API reference

VSH exposes one Rust-owned execution model through two deliberately separate language
surfaces.

## Choose a language

| Surface | Package | Reference |
|---|---|---|
| Python | PyPI distribution `vbash`, import `vsh` | [Python API](python/api.md) |
| Rust | Cargo package `vsh-runtime`, library `vsh` | [Rust API](rust/api.md) |
| MCP | `vbash[mcp]`, stdio tool `vsh_run` | [MCP server](integrations/mcp.md) |

Python is a thin PyO3 adapter; simulation, policy, transaction state, commit, and
recovery do not have a parallel Python implementation. See the
[language parity contract](rust-rewrite/PARITY_CONTRACT.md).

## Shared concepts

- [Transactions and exact promotion](guides/transactions.md)
- [Policies and resource budgets](guides/policies-and-budgets.md)
- [Architecture and trust boundaries](ARCHITECTURE.md)
- [Typed MCP receipt envelope](integrations/mcp.md#output-envelope)

!!! note "Version 0.3 compatibility break"

    Legacy command-registry, plan-token, shell-rendering, CodeMode discovery, and
    Python committer APIs are not part of the 0.3 native surface. Follow the
    [migration guide](rust-rewrite/MIGRATION.md) instead of importing removed modules.
