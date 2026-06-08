# Changelog

All notable changes to this project are documented in this file.

## [Unreleased]

### Fixed

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
