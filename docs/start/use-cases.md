# Use cases

VSH is most valuable when a program needs broad workspace visibility but its final
filesystem effects must remain bounded, explainable, and atomic enough to recover.

## Coding-agent edits

Let an agent produce one Monty program for a multi-file refactor. VSH returns a compact
receipt or a full canonical change list before any host mutation.

**Useful when:** the agent may touch an uncertain set of files, approvals must bind to
exact output, or the workspace can change between planning and application.

**Recommended flow:** MCP `preview` → inspect `decision`, `changes`, and `risk_flags` →
promote the returned `transaction` with `mode="auto"` only when it is auto-approved.

## Repository migrations

Encode deterministic moves, rewrites, generated files, and cleanup as one program.
Reads influence transaction identity, so a concurrent edit invalidates stale output
instead of being silently overwritten.

Use `ReceiptDetail.FULL` during development and review. Switch to `COMPACT` in high-rate
automation when a diff digest and counts are enough.

## Build and code generation

Generate manifests, configuration, or source files without exposing a real output tree
to the generator. Preview makes output reviewable; auto mode avoids a separate round
trip for small changes that balanced policy deterministically approves.

## Governed local automation

Strict and paranoid profiles turn every mutation into an approval boundary and reduce
hard-denial ceilings. The trusted host can implement its own identity check before
calling the explicit SDK `approve` method.

## IDE and MCP integrations

The stdio server exposes one normal tool, `vsh_run`. A small tool schema reduces agent
context cost while still supporting multi-file programs, policy selection, budgets,
preview promotion, and structured receipts.

## Batch analysis with typed results

Monty code may read many files and return lists, dictionaries, strings, numbers, bytes,
or nested values. Python receives native Python objects through Monty's typed PyO3
converter; Rust receives `MontyObject`. No JSON serialization is required at the SDK
boundary.

## When not to use VSH

Choose another tool when the job requires:

- arbitrary subprocess execution or a POSIX shell;
- network access inside the guest program;
- unrestricted CPython packages or native extensions;
- full container, VM, kernel, or tenant isolation;
- mutation outside one explicitly rooted workspace;
- long-running compute rather than bounded filesystem automation.

VSH also does not decide human intent. Its built-in policy is deterministic; an
organization that needs human or model judgement should place that decision outside
the runtime, then approve the exact pending transaction through a trusted SDK surface.

## Selection checklist

| Question | If yes |
|---|---|
| Do you need to inspect effects before mutation? | Use preview-first transactions |
| Can inputs drift before apply? | Promote the exact transaction; rely on revalidation |
| Do Python and Rust hosts need identical behavior? | Use the shared native runtime |
| Is agent tool-schema size important? | Use the single-tool MCP server |
| Does code need shell/network/ambient environment? | VSH is intentionally not the right boundary |
