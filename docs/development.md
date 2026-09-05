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

## Build the current native surface

This development tree identifies as `0.5.0` and includes the Monty 0.0.22 functions
and September 5 optimizations. From a checkout with the environment bootstrapped,
build matching optimized artifacts:

```bash
cargo build --release --locked -p vsh-monty-worker
uv run --no-sync maturin develop --release --locked --skip-install
export VSH_MONTY_WORKER="$PWD/target/release/vsh-monty-worker"
uv run --no-sync python examples/native/workflows.py
uv run --no-sync python examples/native/mcp_workflow.py
uv run --no-sync python examples/native/cli_workflow.py
cargo run --release --locked -p vsh-runtime --example staged_release
```

For this mixed Python layout, Maturin's `--skip-install` builds the extension in place
without reinstalling dependency groups. It does not build/deploy the separate worker;
the first command and explicit worker path are required. Do not mix a debug extension
with a release native harness when evaluating optimizations. A frozen environment
already containing build requirements can add `--offline` to the Maturin command.

## Documentation

Preview locally:

```bash
python scripts/generate_llms_txt.py
uvx --from zensical==0.0.57 zensical serve
```

Build with CI-equivalent validation:

```bash
python scripts/generate_llms_txt.py --check
uvx --from zensical==0.0.57 zensical build --clean --strict
python scripts/check_docs.py
```

Page source lives under `docs/`, navigation and theme configuration in
`zensical.toml`, and VSH-specific presentation in:

- `docs/assets/stylesheets/vsh.css`;
- `docs/assets/javascripts/copy-markdown.js`;
- `docs/assets/mark.svg`;
- `theme/partials/header.html`, `theme/partials/content.html` and `theme/404.html`.

The Venus palette pairs warm paper and copper accents with a charcoal dark theme.
The vertical navigation, search, theme switching, and instant navigation remain owned
by Zensical; the two template overrides provide a compact header and an in-flow page
toolbar. Keep both overrides compatible with the pinned Zensical version when upgrading.

Every documentation page receives a **Copy as Markdown** action. Its exact source is
bundled in `docs/assets/markdown.json`, generated alongside `llms.txt` and
`llms-full.txt`. The corpus is fetched only on the first copy, from the same site,
so local previews and deployed pages copy their own version without contacting GitHub.
Regenerate these three artifacts after changing Markdown or navigation. Clipboard
errors offer a retry and are announced to assistive technology.

The commands above build only documentation, without installing the project or
compiling Rust. The Pages workflow also watches `theme/` changes. When changing the
layout, check desktop and mobile navigation, both themes, search, content tabs, and
Markdown copying after instant navigation.

The dependency-free `check_docs.py` gate validates local links and fragments, deployed
LLM/source bundles, one copy-source mapping per document and Python snippet syntax.
It runs in the docs-only Pages workflow without installing VSH or compiling Rust.

## Python gates

```bash
uv run ruff check
uv run ruff format --check
uv run ty check \
  src/vsh/__init__.py src/vsh/_version.py src/vsh/cli.py src/vsh/hooks.py \
  src/vsh/mcp src/vsh/pydantic_ai.py src/vsh/_judge.py \
  tests/test_main.py tests/test_native_binding.py tests/test_native_runtime.py \
  tests/test_pydantic_ai_capability.py tests/test_commit_judge.py tests/test_python_surface.py \
  release/check_versions.py release/smoke_wheel.py release/validate_artifacts.py \
  examples/native benchmarks/native_pyo3.py benchmarks/process_tree.py benchmarks/compare.py
uv run basedpyright
uv run pytest \
  tests/test_main.py \
  tests/test_native_binding.py \
  tests/test_native_runtime.py \
  tests/test_pydantic_ai_capability.py \
  tests/test_commit_judge.py \
  tests/test_python_surface.py \
  --cov=src/vsh --cov-branch --cov-report=term-missing --cov-fail-under=100
```

## Rust gates

```bash
cargo fmt --all -- --check
cargo test --workspace --all-features --all-targets --locked
cargo llvm-cov \
  --workspace --all-features --all-targets --locked --summary-only \
  --ignore-filename-regex '(vsh-python|vsh-worker)' \
  --fail-under-lines 79 \
  --fail-under-functions 70 \
  --fail-under-regions 81
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked --exclude vsh-runtime
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

- Describe behavior owned by the current native runtime.
- Mark local measurements with platform, date, sample count, and scope.
- Never present driver RSS as whole-worker-tree RSS.
- Keep Python and Rust signatures separate even when behavior is shared.
- Use `preview` in first examples; explain exact transaction promotion before auto mode.
- Link deep security claims to the threat model or guarantee record.
- Keep published-release and later checkout-only capabilities visibly distinct.
- Test fixture-owning examples as executable contracts.
- Report RSS scope, sampling gaps, confirmation runs and unfavorable benchmark outcomes.

The explicit type-check command above matches the maintained release surface used by CI.
