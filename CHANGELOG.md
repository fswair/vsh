# Changelog

All notable changes to this project are documented in this file.

## [Unreleased]

### Fixed

- JEV tutorials and the preview-only example now use Pydantic AI's standard
  TypeSafe provider and `TYPESAFE_API_KEY`, without a test-only endpoint override.

## [0.6.0] - 2026-10-03

### Added

- `DecisionCommitJudge` for JEV categorical review through Pydantic AI, with
  explicit host instructions, content-sharing authorization, a configurable 0.69
  confidence threshold, and fail-closed decisions. Includes a guided
  synthetic-preview tutorial.
- Opt-in bounded Bash execution through a separate Bashkit worker, with a shared
  policy-aware filesystem gateway, binary-safe output, explicit language selection,
  cancellation and bounded execution evidence. Rust, Python, CLI, MCP and Pydantic AI
  surfaces retain the same preview/review/commit workflow. See the Bash guide for
  supported commands, Unix-only availability and intentionally unsupported features.
- Virtual permission changes and opaque symlinks with canonical diffs, policy
  review, stale revalidation and durable commit/recovery support.

### Fixed

- Commit judges now receive path-bound resolved before/after text and bounded line
  diffs alongside canonical evidence, making removed content explicit without
  changing approval thresholds or content-sharing permissions. Missing evidence or
  exceeded rendering budgets remains pending review; metadata-only review is preserved.
- Commit judges now fail closed before model invocation when a file-content write
  references uncaptured bytes, including transient versions hidden by a safe final
  diff. Native canonical/read completeness alone is not treated as proof that every
  intermediate write can be reviewed.
- Protected wildcard rules now match filenames containing literal `*` characters.
  The policy digest advances to v3; approval, preparation, hook resolution and direct
  commit reject artifacts evaluated under a different policy. Create a fresh preview
  after upgrading. Recovery of already-entered durable commits remains available.

### Changed

- Pydantic AI integration now pins version 2.52.0; the optional `typesafe` extra
  pins TypeSafe SDK 0.7.2. Python imports remain `vsh`; distributions remain
  `vsh-python` with metadata-only `vbash` mirrors.
- Judges require path-bound evidence for every written version. Legitimate net-zero
  writes and temporary-file-and-rename operations such as Bash `sed -i` may therefore
  require application-owned review when their intermediate content is not captured.
- Canonical diffs borrow candidate paths, avoid repeated expansion of nested deletion
  tombstones and avoid temporary node/path clones. Lazy materialization, metrics and
  canonical ordering are preserved.
- Durable state records use one framed write instead of three, preserving their
  existing bytes, checksums, bounds, locks and synchronization guarantees.
- Existing blob directories are inspected before creation, avoiding redundant
  `mkdir` calls while preserving no-follow and opened/named identity checks.
- Documented a scoped stable-integration performance exception: delete-5K p95
  regressed 5.74% against the pre-Bash reference, beyond the unchanged 5% target;
  its median improved 3.44%. Earlier unfavorable measurements remain documented.

## [0.5.0] - 2026-09-06

### Added

- Focused, executable examples for preview, one-shot auto commit, strict independent
  review, bounded analysis, CLI, MCP, and compound native transactions.
- Evidence-first Rust and Python commit hooks with immutable request events, canonical
  diffs, ordered effects, configurable review scope, and fail-closed resolution.
- An exact-pinned Pydantic AI 2.40 capability exposing the native VSH filesystem
  surface and returning pending-review feedback to the calling agent.
- An optional Pydantic AI `CommitJudge` with transaction-bound before/after content,
  explicit content-sharing permission, bounded model calls and direct approval of
  pending transactions. Capability results withhold guest output until committed and
  carry both review and rejection feedback.
- `CommitJudge(review_instructions=...)` extends its fixed evidence-first instructions
  and exposes the VSH adapter explicitly as `judge.hook_handler`.
- `CommitJudge(max_output_tokens=...)` owns the provider output cap; it defaults to
  2,048 and accepts an explicit `None` for backends that reject that request parameter.
- The agent-visible `vsh_run` contract now names its exact Monty functions and Python
  call syntax, preventing invented `read_file` calls and JSON-shaped positional calls.
- `VshCapability(workspace, ...)` is now the sole capability construction surface;
  the redundant `VshCapability.open(...)` constructor was removed before release.

### Removed

- The retired source-only Python command engine and its stale tests, agent examples,
  playground benchmark archive, rewrite-phase notes, and obsolete implementation plans.

## [0.4.0] - 2026-09-05

### Changed

- Reduced policy/path allocation churn, specialized common protected-path patterns,
  fused snapshot indexing, and bounded overlay child lookups without changing
  transaction identities, authorization order, revalidation or commit durability.
- Rebuilt the user documentation around native Rust/Python workflows, explicit
  preview retention and trust boundaries, executable recipes and dated optimization evidence.
- Upgraded the exact-pinned Monty release train from `0.0.21` to `0.0.22`, including
  its Ruff `0.0.9`, `compact_str 0.10`, and `get-size2 0.10.3` graph.

### Added

- Dependency-free policy/path microbenchmarks, process-tree RSS sampling and generated
  release-baseline/final/confirmation comparisons under `benchmarks/results/2026-09-05`.
- Fixture-owning Python, Rust, MCP and separate-process CLI cookbook acceptance examples.
- Active-snapshot Monty functions for read, write, list, mkdir, remove, move, copy,
  glob, literal search, and exact text patch operations

### Fixed

- Documentation CTA hover/focus/pressed contrast in both Venus themes.
- APFS non-UTF-8 filename test setup now recognizes Darwin's pre-capture EILSEQ rejection.
- Fresh crates.io consumers retain the Monty-compatible `get-size2 =0.10.3`
  resolution selected by Monty 0.0.22 and Ruff 0.0.9.
- Case-insensitive VSH search reports original Unicode columns without using folded
  byte offsets against the source string.

## [0.3.1] - 2026-09-02

### Changed

- Promoted `vsh` to the primary crates.io facade while retaining `vsh-runtime` as the
  implementation crate.
- Renamed the native PyPI distribution to `vsh-python`; the import package and CLI
  remain `vsh`.
- Converted both `vbash` registry handles into exact-version compatibility mirrors:
  crates.io re-exports `vsh`, while PyPI contains metadata only and installs
  `vsh-python`.
- Advanced the lockstep release version to `0.3.1` because PyPI release files are
  immutable and the published `vbash==0.3.0` payload cannot be replaced.

### Added

- Protected workspace path policy (`VSH_PROTECTED_PATTERNS`, `VSH_PROTECTED_PATTERNS_FILE`)
- Approval tiers on `SimulationResult` (`read_only`, `mutation`, `destructive`)
- `auto_approve_plan()` for read-only auto-approval; `approve_plan(..., auto=True)`
- Read command stdout capture in `ActualEffects.stdout` (`vsh.execute.read_output`)
- Execution timing fields: `simulation_time_ms`, `execution_time_ms`, `total_time_ms`
- `VSH_MAX_TOUCHED_PATHS` simulation limit (default `500`)
- Shell previews quote paths with `shlex.quote`

### Fixed

- Fresh crates.io consumers retain the Monty-compatible `get-size2 =0.10.1`
  resolution instead of selecting the incompatible 0.10.3/Ruff combination.
- `vsh_list` now rejects workspace escapes (`/`, `..`, paths outside the root) during simulation
- Execution layer enforces workspace boundaries via `ExecutionContext.resolve_within_workspace`

## [0.2.0] - 2026-06-08

### Added

- CodeMode FastMCP server (`vsh serve-codemode`, `vsh-codemode`) with workflow prompts
- CodeMode custom instructions via CLI (`-i`, `-f`), env vars, and `create_codemode_server(custom_instructions=...)`
- Real filesystem executor (`vsh.execute.apply_command`, `execute_approved`)
- Pre-execution drift detection via path fingerprints (`vsh.execute.revalidate_plan`)
- Plan fingerprints and touched-path basis on every simulation
- Optional JSON persistence (`VSH_DATA_DIR`, `VSH_PERSIST`)
- Extension hook registry for hydration, semantic analyzers, shadow workspaces
- `vsh_sort` command
- Gemini-safe inlined JSON schemas (`inline_json_schema`)
- Documentation: `docs/ARCHITECTURE.md`, `docs/API.md`, `docs/CODEMODE.md`

### Changed

- `shell_preview` now uses `command.to_shell()` instead of `repr(command)`
- `approve_with_warning` plans are execution-eligible (still gated by raw shell match)
- `ExecutionResult` includes `revalidation`, `actual_effects`, `matches_prediction`
- Snapshot builder ignores `.venv`, `node_modules`, `dist`, `build`, `target`
- Version bumped to `0.2.0` (23 commands)

### Fixed

- pydantic-ai / Gemini tool schema failures from `$defs/SideEffect` references

## [0.1.0] - 2026-06-08

First public v0 release.

### Added

- Typed command registry with JSON schema export for workspace commands
- `StructuredCommand` models with canonical shell previews and side-effect metadata
- Workspace snapshot graph builder with protected-root policy
- Simulation engine with approve / reject / warn decisions
- Plan store, approval tokens, and execution eligibility checks
- FastMCP server exposing search, schema, snapshot, simulate, approve, and execute tools
- CLI: `vsh search`, `vsh schema`, `vsh names`, `vsh serve`
- pydantic-ai `FunctionToolset` integration (`vsh.agent`)
