# CodeMode server

`vsh-codemode` is the agent-oriented stdio entry point in VSH 0.3. It exposes the same
single `vsh_run` tool as `vsh serve`, then adds server instructions and the
`vsh_run_transaction` MCP prompt.

```bash
uv run vsh-codemode
```

The legacy multi-tool CodeMode surface was removed during the Rust rewrite. One tool
keeps discovery and prompt cost small without maintaining a parallel command registry.
The agent supplies one bounded Monty program; Rust owns snapshot, simulation, canonical
diff, policy, state, revalidation, commit, and recovery.

Project rules can be appended without forking the server:

```bash
VSH_CODEMODE_INSTRUCTIONS_FILE=.vsh/agent-rules.md uv run vsh-codemode
```

or with `VSH_CODEMODE_INSTRUCTIONS` for a short inline rule. File content is placed
before inline content when both variables are set.

Use the current guides:

- [MCP server setup and `vsh_run` schema](integrations/mcp.md)
- [Agent protocol and failure behavior](integrations/agents.md)
- [Migration from the legacy Python engine](rust-rewrite/MIGRATION.md)

Do not configure removed tools such as `search`, `get_schema`, `simulate`, `approve`,
`execute_approved`, or `vsh_sandbox`. Independent approval remains available through a
trusted Python or Rust SDK host and is intentionally absent from the model-facing MCP
surface.
