# Rust cookbook

Rust uses the native `vsh` facade directly. The application owns capabilities,
configuration and review; Monty owns only the bounded virtual program. No Python host
interpreter or PyO3 crossing is required.

## Run an end-to-end staged release

From the current source checkout:

```bash
cargo build --release --locked -p vsh-monty-worker
VSH_MONTY_WORKER="$PWD/target/release/vsh-monty-worker" \
  cargo run --release --locked -p vsh-runtime --example staged_release
```

This repository example belongs to the `vsh-runtime` implementation package in
`crates/vbash`; external applications still depend on `vsh`. It uses a unique temporary
workspace, not the repository. No extra dependency or model credential is required.

The [Rust host source](https://github.com/fswair/vsh/blob/main/crates/vbash/examples/staged_release.rs)
embeds the same [guest program](https://github.com/fswair/vsh/blob/main/crates/vbash/examples/staged_release.monty)
as the [Python recipe](../python/examples.md#staged-release-generation). Both are
included with the VSH 0.5.0 source release.

The workflow creates a template, virtually copies and patches it, renames the generated
config and writes a README. It checks the exact three final paths and that no host
release directory exists. The rename produces pending approval. A clearly labeled
trusted fixture reviewer approves, then the host commits the exact transaction and
checks both output contents and absence of the intermediate file.

## Understand the native calls

These fragments come from that complete example:

```rust
let runtime = Runtime::open(RuntimeConfig::new(&workspace.0))?;
let code = include_str!("staged_release.monty");
let preview = runtime.preview(
    RunRequest::new(code).with_detail(ReceiptDetail::Full),
)?;
```

`RunRequest` borrows source; `Runtime::preview` returns owned evidence. Match the
decision enum rather than parsing text:

```rust
assert!(matches!(preview.decision, RuntimeDecision::PendingApproval(_)));
assert!(!workspace.0.join("release").exists());
```

After authenticating a reviewer in a real application, use a principal digest and
Unix-millisecond approval interval:

```rust
runtime.approve(
    preview.transaction,
    PrincipalId::digest_label("fixture-reviewer"),
    now,
    now + 30_000,
)?;
let committed = runtime.commit(preview.transaction, now)?;
assert!(committed.commit.is_some());
```

A label digest is identity binding, not authentication. Denied transactions cannot be
approved, and approval does not waive stale checks or grant permission to rerun source.

## Analysis without application

For a read-only request, return a small `MontyObject` result and discard its
auto-approved artifact after consuming it:

```rust
let receipt = runtime.preview(RunRequest::new("{'answer': 42}"))?;
println!("{:?}", receipt.value);
runtime.discard_preview(receipt.transaction)?;
```

Native `ReceiptDetail::Full` includes `DiffEntry` before/after `NodeState` values.
These are typed content identities and metadata, not a ready-made unified text diff.
Keep content evidence bounded and explicit.

## Budgets and deployment

Use struct-update syntax with `ExecutionBudget::default()` to change only owned limits.
`RuntimeConfig` additionally exposes snapshot, artifact, store and commit limits that
the Python convenience constructor does not. An idle-worker pool size is not a cap
on all concurrent application requests; add admission control at the service boundary.

Always use the supervised worker for agent-authored source. The in-process harness
removes crash isolation and worker heap enforcement and is not a production speed trick.
See [Rust setup](index.md), [API reference](api.md) and [performance](../performance.md).
