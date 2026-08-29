# VSH

VSH is one low-latency transactional filesystem simulator with two SDK surfaces:

- native Rust through the `vsh-runtime` package (`vsh` library target),
- Python through the `vbash` PyPI distribution and `vsh` import package.

Both surfaces execute the same Rust pipeline. Python is a thin PyO3 binding; it does
not contain a fallback simulator or committer.

```text
Monty program → immutable snapshot → Rust VirtualFs → canonical diff → policy
              → dependency revalidation → recoverable commit → verified receipt
```

VSH is not a POSIX shell and never gives Monty a host filesystem mount, subprocess,
network, or ambient environment capability.

## Python

Install the wheel:

```bash
uv add vbash
```

Run one complete transaction. `PREVIEW` guarantees that host files are unchanged;
`AUTO` commits only a deterministic native auto-approval.

```python
from vsh import ReceiptDetail, RunMode, RunRequest, Runtime

runtime = Runtime.open("/path/to/workspace")
receipt = runtime.run(
    RunRequest(
        """
from pathlib import Path
text = Path('/workspace/input.txt').read_text()
Path('/workspace/output.txt').write_text(text.upper())
len(text)
""",
        mode=RunMode.AUTO,
        detail=ReceiptDetail.FULL,
    )
)

print(receipt.state, receipt.result, receipt.changes)
```

The GIL is released while snapshotting, executing Monty, generating the diff,
evaluating policy, revalidating dependencies, and committing.

## Rust

Until the first registry release, use the workspace package directly:

```toml
[dependencies]
vsh-runtime = { path = "crates/vsh-sdk", version = "=0.3.0" }
```

```rust,no_run
use vsh::{ReceiptDetail, RunMode, RunRequest, Runtime, RuntimeConfig};

fn main() -> Result<(), vsh::VshError> {
    let runtime = Runtime::open(RuntimeConfig::new("/path/to/workspace"))?;
    let receipt = runtime.run(
        RunRequest::new(
            "from pathlib import Path\nPath('/workspace/output.txt').write_text('ok')",
        )
        .with_mode(RunMode::Auto)
        .with_detail(ReceiptDetail::Full),
    )?;
    println!("{:?} {}", receipt.state, receipt.transaction);
    Ok(())
}
```

Production execution expects the matching `vsh-monty-worker` executable. Python
wheels bundle it; native embedders configure its trusted path through
`RuntimeConfig::with_worker_path`.

## MCP

Install the optional adapter and start the stdio server:

```bash
uv add "vbash[mcp]"
vsh serve
```

The server exposes exactly one normal tool, `vsh_run`. A multi-file operation is one
Monty program, one Rust transaction, one policy decision, and one Python-to-Rust call.
An auto-approved preview can later be promoted by passing its returned transaction
handle with `mode="auto"`; VSH revalidates dependencies before any mutation.

Auto-approved previews use a bounded process-local fast path and must be promoted by
the same live `Runtime` (or MCP server process). The exact artifact is made durable
before reservation and commit. Transactions that require independent approval are
durable immediately and survive a runtime restart.

## Security and dependency policy

- External Rust crates are exact-pinned in the workspace manifest and lockfile.
- Unknown, abandoned, yanked, Git-only, wildcard, and unreviewed crates are rejected.
- `cargo audit` and `cargo deny` are release gates.
- The hash-locked optional/development Python graph is a strict `pip-audit` gate.
- Protected paths are denied before names or bytes enter Monty.
- Approval binds the exact program, snapshot, read/write dependencies, canonical diff,
  policy, execution configuration, and intent.
- The committer is the only host writer and serializes only same-workspace commit
  revalidation/mutation; independent runtimes do not share a global lock.
- Process-local preview retention is fail-closed and bounded by both artifact count and
  encoded bytes; it cannot become an unbounded parent-process cache.

See [the threat model](docs/rust-rewrite/THREAT_MODEL.md),
[guarantees](docs/rust-rewrite/GUARANTEES.md), and
[dependency record](docs/rust-rewrite/DEPENDENCY_POLICY.md). Upgrade and benchmark
details are in the [migration guide](docs/rust-rewrite/MIGRATION.md) and
[performance record](docs/rust-rewrite/PERFORMANCE.md). Merge coverage scope and floors
are recorded in the [coverage contract](docs/rust-rewrite/COVERAGE.md). The cross-platform build,
validation, provenance, and dual-registry order are in the
[release guide](docs/rust-rewrite/RELEASE.md).

## Documentation

The detailed Rust, Python, MCP, agent, architecture, security, and benchmark guides are
built with the exactly pinned Zensical toolchain:

```bash
uv run python scripts/generate_llms_txt.py
uv run zensical serve
uv run zensical build --clean --strict
```

Start at [`docs/index.md`](docs/index.md). The site includes a VSH-specific dark-red
design, automatic light/dark palette selection, and a **Copy as Markdown** action on
every page. It also publishes a compact LLM index at `/llms.txt` and the complete
Markdown source corpus at `/llms-full.txt`, with navigation pages first; CI verifies
both generated files are current.

## Development

```bash
uv sync --frozen --all-groups --extra mcp
cargo fmt --all -- --check
cargo test --workspace --all-targets --locked
cargo llvm-cov --workspace --all-features --all-targets --locked --summary-only \
  --ignore-filename-regex '(vsh-python|vsh-worker)' \
  --fail-under-lines 79 --fail-under-functions 70 --fail-under-regions 81
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
uv run ruff check
uv run ruff format --check
uv run pytest \
  tests/test_native_binding.py tests/test_native_runtime.py tests/test_python_surface.py \
  --cov=src/vsh --cov-branch --cov-report=term-missing --cov-fail-under=100
cargo run --release --locked -p vsh-runtime --example native_benchmark -- \
  --worker "$PWD/.venv/bin/vsh-monty-worker"
```

## License

Apache-2.0
