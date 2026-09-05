# VSH

VSH is one low-latency transactional filesystem simulator with two SDK surfaces:

- native Rust through the `vsh` crate, backed by `vsh-runtime`,
- Python through the `vsh-python` PyPI distribution and `vsh` import package.

Both surfaces execute the same Rust pipeline. Python is a thin PyO3 binding; it does
not contain a fallback simulator or committer.

Special thanks to [Artyom Pavlov](https://github.com/newpavlov) for generously
donating the `vsh` crate name to this project.

```text
Monty program → immutable snapshot → Rust VirtualFs → canonical diff → policy
              → dependency revalidation → recoverable commit → verified receipt
```

VSH is not a POSIX shell and never gives Monty a host filesystem mount, subprocess,
network, or ambient environment capability.

## Python

Install the wheel:

```bash
uv add vsh-python
```

Run one complete transaction. `PREVIEW` does not apply changes to user workspace files;
the host runtime may still persist bounded transaction/storage artifacts. `AUTO`
commits only a deterministic native auto-approval.

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

```toml
[dependencies]
vsh = "=0.4.0"
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

## Active-snapshot filesystem functions

VSH 0.4.0 injects ten bounded `vsh_*` functions into each Monty program: read, write,
list, make directory, remove, move, copy, glob, literal search and exact text patch.
They share the same copy-on-write snapshot as `pathlib`, so earlier virtual writes are
visible to later calls without creating a nested runtime or crossing MCP.

```python
files = vsh_glob('**/*.toml', path='/workspace/services', max_results=101)
assert len(files) <= 100, 'split this migration into reviewed batches'
for path in files:
    assert vsh_patch(path, 'timeout = 5', 'timeout = 15', count=1) == 1
{'updated': len(files)}
```

## MCP

Install the optional adapter and start the stdio server:

```bash
uv add "vsh-python[mcp]"
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

## Performance

On the dated macOS arm64 release-profile matrix, VSH 0.4.0 lowers median latency by
roughly 25–30% for 10,000-file discovery/glob and 5,050-entry removal workloads. The
whole benchmark harness reports about 19% less CPU time. Small-call variance remains,
and sampled process-tree RSS does not establish a repeatable memory reduction. See the
[protocol, complete results and caveats](https://fswair.github.io/vsh/performance/).

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

See the [threat model](https://fswair.github.io/vsh/rust-rewrite/THREAT_MODEL/),
[guarantees](https://fswair.github.io/vsh/rust-rewrite/GUARANTEES/), and
[dependency record](https://fswair.github.io/vsh/rust-rewrite/DEPENDENCY_POLICY/).
Upgrade and benchmark details are in the
[migration guide](https://fswair.github.io/vsh/rust-rewrite/MIGRATION/) and current
[performance report](https://fswair.github.io/vsh/performance/). Merge coverage scope
and floors are recorded in the
[coverage contract](https://fswair.github.io/vsh/rust-rewrite/COVERAGE/).

## Compatibility handles

Existing declarations may continue to install `vbash==0.4.0` from PyPI or depend on
`vbash = "=0.4.0"` from crates.io. Both packages are implementation-free mirrors of
the exact matching VSH release. New projects should use `vsh-python` and `vsh`
directly; Python code continues to write `import vsh` either way.

## Documentation

The detailed Rust, Python, MCP, agent, architecture, security, and benchmark guides are
built with the exactly pinned Zensical toolchain:

```bash
uv run python scripts/generate_llms_txt.py
uv run zensical serve
uv run zensical build --clean --strict
```

Start at the [VSH documentation](https://fswair.github.io/vsh/). The site includes a
Venus-inspired palette, automatic light/dark selection, and a **Copy as Markdown** action on
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
