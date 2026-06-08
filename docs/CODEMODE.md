# vsh CodeMode MCP server

`vsh-codemode` is a FastMCP server that exposes the same compact vsh surface as
`vsh serve`, but is explicitly designed for **CodeMode-style agent discovery**.

## Why CodeMode?

CodeMode-style tooling keeps the MCP surface small. Instead of stuffing every command
schema into the model context up front, the agent:

1. **searches** for the capability it needs
2. **fetches one schema** on demand
3. **acts** with typed parameters

That pattern reduces prompt bloat, avoids stale tool definitions, and mirrors how
agents should explore unfamiliar APIs: discover first, then specialize.

`vsh` takes that idea and adds what raw CodeMode-style discovery usually lacks:

| CodeMode-style step | vsh addition |
|---------------------|--------------|
| search | `CommandSpec` cards with tags, examples, mutates_fs |
| get_schema | Gemini-safe inlined JSON schema |
| — | `snapshot_workspace` for a stable workspace graph |
| — | `simulate` with journal + predicted effects |
| — | `approve` as an immutable plan gate |
| — | `execute_approved` with drift revalidation |

So the inspiration is CodeMode discovery; the product difference is **validation-first
execution**.

## Start the server

```bash
vsh serve-codemode
```

Equivalent entry points:

```bash
vsh-codemode
python -m vsh.mcp.codemode_server
```

Default transport is stdio — use it from Cursor, Claude Desktop, Codex, or any MCP
client that launches local servers.

## Tool surface (compact on purpose)

| Tool | Role in CodeMode flow |
|------|------------------------|
| `search` | Find command families by keyword |
| `get_schema` | Pull one JSON schema when needed |
| `snapshot_workspace` | Capture workspace graph once per session |
| `simulate` | Dry-run typed params against the snapshot |
| `approve` | Promote a plan to an approval token |
| `execute_approved` | Apply an approved, revalidated plan |

Resources carry stateful context:

| Resource | Purpose |
|----------|---------|
| `workspace://snapshot/current` | Latest snapshot summary |
| `workspace://projection/current` | cwd-oriented tree projection |
| `commands://spec/{name}` | Spec card + schema bundle |
| `simulations://{plan_id}` | Persisted simulation artifact |

## MCP prompts

The CodeMode server also registers workflow prompts:

| Prompt | When to use |
|--------|-------------|
| `vsh_discover_command` | Before simulation — search + schema only |
| `vsh_simulate_and_execute` | Full lifecycle after schema is known |
| `vsh_read_workspace` | Safe read-only inspection flow |

Prompts are optional hints for clients that support MCP prompt listing. The server
`instructions` field repeats the same workflow for clients that read server metadata.

## Custom instructions

CodeMode ships with built-in workflow rules, but you can append **project-specific**
guidance without forking the server. Custom text is merged after the defaults under a
`Project-specific instructions:` section.

### CLI

```bash
vsh serve-codemode -i "Prefer vsh_rg over vsh_grep. Never chmod outside src/."
vsh serve-codemode -f .vsh/codemode-instructions.md
```

Both flags can be combined; file content comes first, then inline text.

### Environment variables

| Variable | Purpose |
|----------|---------|
| `VSH_CODEMODE_INSTRUCTIONS` | Inline extra instructions |
| `VSH_CODEMODE_INSTRUCTIONS_FILE` | Path to a UTF-8 instructions file |

CLI flags take precedence over environment variables when both are set.

### Example project file

Create `.vsh/codemode-instructions.md` in your repo:

```markdown
- Workspace root is the monorepo; simulate from package subdirs only.
- Use vsh_list before destructive commands.
- Do not execute plans that touch .env or credentials.
```

Cursor config with a file-backed server:

```json
{
  "mcpServers": {
    "vsh-codemode": {
      "command": "uv",
      "args": ["run", "vsh", "serve-codemode", "-f", ".vsh/codemode-instructions.md"],
      "cwd": "/path/to/your/workspace/repo"
    }
  }
}
```

### Python embedding

```python
from vsh.mcp import build_codemode_instructions, create_codemode_server

server = create_codemode_server(
    custom_instructions="Only mutate files under packages/api/."
)
server.run()
```

Use `build_codemode_instructions()` when you need the merged string without starting
the server.

## Recommended agent loop

```text
search("list")
  -> pick vsh_list

get_schema("vsh_list")
  -> build params from schema/examples

snapshot_workspace()
  -> keep snapshot_id

simulate("vsh_list", snapshot_id, params)
  -> inspect decision + predicted_effects

approve(plan_id)
  -> approval_token

execute_approved(approval_token)
  -> applied + actual_effects
```

## Cursor / Claude Desktop config sketch

```json
{
  "mcpServers": {
    "vsh-codemode": {
      "command": "uv",
      "args": ["run", "vsh-codemode"],
      "cwd": "/path/to/your/workspace/repo"
    }
  }
}
```

If the project virtualenv is already active:

```json
{
  "mcpServers": {
    "vsh-codemode": {
      "command": "vsh-codemode"
    }
  }
}
```

## Compared to `vsh serve`

| | `vsh serve` | `vsh serve-codemode` |
|---|-------------|----------------------|
| Tools/resources | same 6 + 4 | same 6 + 4 |
| Server name | `vsh` | `vsh-codemode` |
| Server instructions | default | CodeMode workflow + rules |
| MCP prompts | none | discovery / simulate / read |
| Use case | general MCP embedding | agent discovery-first workflows |

Pick `serve-codemode` when the client should steer the model toward discovery-first
behavior without embedding vsh docs in the system prompt.

## Python embedding

```python
from vsh.mcp import create_codemode_server, run_codemode_server

# Default CodeMode instructions only
server = create_codemode_server()
server.run()

# Or resolve env / file / inline at runtime
run_codemode_server(instructions_file=".vsh/codemode-instructions.md")
```

## Related docs

- [Architecture](ARCHITECTURE.md)
- [API reference](API.md)
