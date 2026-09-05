# CodeMode server

`vsh-codemode` is the agent-oriented stdio entry point in VSH 0.3. It exposes the same
single `vsh_run` tool as `vsh serve`, then adds server instructions and the
`vsh_run_transaction` MCP prompt.

```bash
uv run vsh-codemode
```

The legacy multi-tool CodeMode surface was removed during the Rust rewrite. One MCP tool
keeps discovery and prompt cost small without maintaining a parallel command registry.
Inside its Monty program, the agent can use `pathlib` plus ten built-in high-level VSH
functions. Both operate on the same active overlay; the built-ins do not expand the MCP
surface or create nested transactions. Rust owns snapshot, simulation, canonical diff,
policy, state, revalidation, commit, and recovery.

The ten functions use the Monty 0.0.22 integration included in VSH 0.4.0. Each outer
program gets a fresh snapshot; CodeMode is not a persistent uncommitted virtual session.

Project rules can be appended without forking the server:

```bash
VSH_CODEMODE_INSTRUCTIONS_FILE=.vsh/agent-rules.md uv run vsh-codemode
```

or with `VSH_CODEMODE_INSTRUCTIONS` for a short inline rule. File content is placed
before inline content when both variables are set.

Use the current guides:

- [MCP server setup and `vsh_run` schema](integrations/mcp.md)
- [VSH functions available inside Monty](integrations/monty-tools.md)
- [Agent protocol and failure behavior](integrations/agents.md)
- [Migration from the legacy Python engine](rust-rewrite/MIGRATION.md)

Do not configure removed tools such as `search`, `get_schema`, `simulate`, `approve`,
`execute_approved`, or `vsh_sandbox`. Independent approval remains available through a
trusted Python or Rust SDK host and is intentionally absent from the model-facing MCP
surface. The in-program `vsh_search` function is part of `vsh_run.code`; it is not a
separate MCP tool.

Keep workspace/profile/budget choices host-owned and manage preview retention. The raw
MCP runtime LRU can evict auto-approved handles and has no discard tool. Instructions
alone do not enforce those deployment controls; see the current integration guides.
