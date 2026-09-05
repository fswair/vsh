# VSH examples

Every example in this directory uses the current Rust-owned transaction API. They run
against disposable temporary workspaces and never mutate the repository.

## Python

Run the focused examples from a checkout with the native extension and matching worker:

```bash
uv run --no-sync python examples/native/preview.py
uv run --no-sync python examples/native/auto_commit.py
uv run --no-sync python examples/native/strict_review.py
uv run --no-sync python examples/native/budgeted_analysis.py
uv run --no-sync python examples/native/commit_judge.py
```

The commit-judge example uses an offline model by default and requires the `pydantic-ai`
extra. An explicit `--model provider:model` selects a configured live provider.

The opt-in live harness uses `codex-auth-helper` for a real Pydantic AI main agent and
separate commit judge:

```bash
uv run --no-sync --with codex-auth-helper==1.7.0 \
  python examples/live_commit_judge.py
```

It runs only in a disposable workspace and covers safe approval, misleading intent,
file-content prompt injection, a missing authentication invariant, config deletion,
and an unauthorized second path. It makes real model calls and is intentionally
excluded from the offline test suite. The helper is installed for this command only;
check its declared Pydantic AI compatibility before adding it to an application.

The larger examples cover compound transactions, stale-input rejection, CLI process
boundaries, and the single-tool MCP server:

```bash
uv run --no-sync python examples/native/workflows.py
uv run --no-sync python examples/native/cli_workflow.py
uv run --no-sync python examples/native/mcp_workflow.py
```

## Rust

The compile-tested Rust host example and its shared Monty program live with the native
runtime crate:

```bash
cargo build --release --locked -p vsh-monty-worker
VSH_MONTY_WORKER="$PWD/target/release/vsh-monty-worker" \
  cargo run --release --locked -p vsh-runtime --example staged_release
```

See `docs/python/examples.md` and `docs/rust/examples.md` for the reasoning and security
rules behind each workflow. Pydantic AI users should start with the deterministic and
judge tutorials under `docs/tutorials/`.
