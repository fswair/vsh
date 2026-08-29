# Development

The workspace builds one Rust core, a PyO3 wheel, a supervised worker, and this Zensical
documentation site.

## Bootstrap

```bash
uv sync --frozen --all-groups --extra mcp
rustup show active-toolchain
```

Rust is pinned in `rust-toolchain.toml`; Python and documentation dependencies are
locked in `uv.lock`. Zensical is exactly pinned to `0.0.57`.

## Documentation

Preview locally:

```bash
uv run zensical serve
```

Build with CI-equivalent validation:

```bash
uv run zensical build --clean --strict
```

Page source lives under `docs/`, navigation and theme configuration in
`zensical.toml`, and VSH-specific presentation in:

- `docs/assets/stylesheets/vsh.css`;
- `docs/assets/javascripts/copy-markdown.js`;
- `docs/assets/mark.svg`.

Every page receives a **Copy as Markdown** action. It fetches the exact page source from
the configured repository action, so keep `repo_url`, `edit_uri`, and deployed branch
aligned.

## Python gates

```bash
uv run ruff check
uv run ruff format --check
uv run ty check
uv run basedpyright
uv run pytest \
  tests/test_native_binding.py \
  tests/test_native_runtime.py \
  tests/test_python_surface.py \
  --cov=src/vsh --cov-branch --cov-report=term-missing --cov-fail-under=100
```

## Rust gates

```bash
cargo fmt --all -- --check
cargo llvm-cov \
  --workspace --all-features --all-targets --locked --summary-only \
  --ignore-filename-regex '(vsh-python|vsh-worker)' \
  --fail-under-lines 79 \
  --fail-under-functions 70 \
  --fail-under-regions 81
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
```

## Security and release gates

```bash
cargo audit --file Cargo.lock
cargo deny --all-features --locked check
uv build
```

The release workflow additionally validates every crate archive, wheel and sdist,
installs wheels in clean interpreters, runs preview/commit smoke tests, checks exact
versions, and publishes only through protected tag-triggered environments.

## Documentation writing rules

- Describe behavior owned by the current native runtime, not removed legacy modules.
- Mark local measurements with platform, date, sample count, and scope.
- Never present driver RSS as whole-worker-tree RSS.
- Keep Python and Rust signatures separate even when behavior is shared.
- Use `preview` in first examples; explain exact transaction promotion before auto mode.
- Link deep security claims to the threat model or guarantee record.
