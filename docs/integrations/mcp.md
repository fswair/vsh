# MCP server

VSH exposes one normal MCP tool, `vsh_run`. Its `code` argument is a complete Monty
program or explicitly host-enabled bounded Bash program: one snapshot, one active
overlay, one canonical change set and one policy
decision. The Python adapter constructs a native request and projects the receipt;
it does not implement a separate filesystem simulator.

!!! note "Development checkout hardening"

    Host-fixed tool configuration, aggregate payload budgets and automatic read-only
    preview cleanup on this page are unreleased changes after 0.6.0. Build this checkout
    to use them; the installation command below installs the published release.

## Install and launch

```bash
python -m pip install 'vsh-python[mcp]==0.6.0'
vsh serve
```

Transport is stdio. `vsh-codemode` exposes the same one-tool surface with built-in
workflow instructions and the `vsh_run_transaction` prompt:

```bash
vsh-codemode
```

The ten in-program VSH functions and the existing `pathlib` surface are included in
the current release. See [source installation](../development.md) when developing from a checkout.

The Bash selector, `VSH_ENABLE_BASH`, and binary stream fields are included in 0.6.0.
Bash remains an explicit Linux/macOS opt-in; see [Bash setup](bash.md#enable-it-explicitly).
Monty is the default and remains supported on Windows.

## Connect a client

For a client supporting this common launch configuration, point at the executable
inside the environment where VSH is installed:

```json
{
  "mcpServers": {
    "vsh": {
      "command": "/absolute/path/to/.venv/bin/vsh",
      "args": ["serve"],
      "cwd": "/absolute/path/to/workspace"
    }
  }
}
```

Use the environment's `vsh-codemode` executable with empty `args` for CodeMode guidance.
Client configuration formats vary. The server captures `VSH_WORKSPACE_ROOT` (or its
startup working directory) as its authorized workspace when registering the tool.
The model cannot replace the root, policy, worker or execution budget. Python hosts
can configure these with `register_vsh_surface(server, workspace_root=..., policy=...,
budget=...)` or `create_codemode_server(...)`. These are host constructor options,
not tool arguments. The Python `native_tools.vsh_run` helper and CLI remain explicit
trusted-host interfaces.

## Request contract

```text
vsh_run(
    code: str | None = None,
    *,
    language: "monty" | "bash" = "monty",
    transaction: str | None = None,
    intent: str | None = None,
    mode: "preview" | "auto" = "preview",
    detail: "compact" | "full" = "compact",
) -> dict[str, object]
```

Bash is disabled by default. Start the server with `VSH_ENABLE_BASH=1` (and an
optional host-only `VSH_BASH_WORKER` path) to advertise `"bash"` in the tool schema.
The guest cannot enable it by passing a language argument. The receipt includes
language, exit/profile and bounded base64 streams with explicit truncation markers.
See [bounded Bash](bash.md) for the supported Unix profile and noncommittable failures.

For new work pass `code`. For promotion pass `transaction`, no code, and `mode="auto"`.
Resolve the same workspace/profile/worker identity as the preview. Promotion does not
rerun source or create a different detail/budget configuration for the existing artifact.
Host-configured budget keys match the [Python execution budget](../python/api.md#executionbudget).

## Preview, review, promote

This fixture program creates one known status file:

```json
{
  "code": "from pathlib import Path\nPath('/workspace/status.txt').write_text('ready\\n')\n'ready'",
  "mode": "preview",
  "detail": "full",
  "intent": "Create the reviewed status fixture"
}
```

Check that the decision is `auto_approved`, the exact path/kind list is expected,
content evidence matches the task, and no output truncation obscures review. Only
then promote through the same retained runtime:

```json
{
  "transaction": "<exact transaction from preview>",
  "mode": "auto"
}
```

Require `commit.committed == true` before reporting that host files changed. A returned
value or `auto_approved` decision alone is insufficient. A stale failure needs a new
proposal and review, not a forced replay.

## Receipt envelope

| Location | Contents |
|---|---|
| Top-level identity | `transaction`, `base_snapshot`, `diff` digest |
| Top-level decision | `state`, `decision`, `risk_flags`, `deny_reason` |
| Top-level changes | `changed_paths`, `changes: [{path, kind}]` in full detail |
| Top-level guest output | `language`, `result_repr`, `result_truncated`, `stdout`, `stdout_truncated`, `stderr`, `stderr_truncated` |
| `bash` | `None` for Monty; otherwise `profile`, `exit_code`, `encoding="base64"`, encoded `stdout`/`stderr` and `output_truncated` |
| `execution` | `os_calls`, `read_bytes`, `write_bytes`, `directory_entries`, `output_bytes`, `denied_accesses`, `result_bytes` |
| `commit` | `committed`, `operations`, `verified_paths`, `cleanup_pending` |
| `timings_ns` | `snapshot`, `execute`, `diff`, `policy`, `bind_and_store`, `commit`, `total` |

MCP returns `result_repr`, not the Python SDK's arbitrary typed `result`. Do not `eval`
that representation. `diff` is not a textual diff; request bounded before/after content
when reviewing a transformation.

The registered agent tool has an aggregate **16 KiB compact ASCII-JSON payload budget**,
including escaping and field names. The host can set `max_response_bytes` when registering
the surface (minimum 4096). This is a byte budget, not a token guarantee or a bound on
MCP transport wrappers. Large result/output/change-list display fields are omitted with
`response_truncated=true`; state, transaction identity and commit outcome remain truthful.
Return selected fields or short slices from Monty when you need useful bounded output.
Bash streams appear once as base64 in `bash`, not again as repr and lossy text.
The trusted Python helper and CLI retain their separate per-field 65,536-character/raw-byte
display limits; they do not claim the registered tool's aggregate bound.

## Lifetime and retention limits

### Cancellation

The registered MCP tool offloads blocking native work without detaching it. An MCP
`notifications/cancelled` notification received before native commit entry cancels
execution and joins cleanup. Unseen previews are discarded or their durable automatic
approval is revoked. The synchronous Python helper `vsh.mcp.vsh_run` remains available
for ordinary local calls; the server registers a cancellation-aware async adapter.

Clients must send the protocol cancellation notification. Abandoning a local await or
closing a UI does not necessarily notify the server. Once native commit entry wins the
race, the commit completes with its actual outcome; cancellation does not roll it back.
A cancelled transport may no longer deliver that result, so reconcile transaction
state through your trusted host instead of assuming host files were untouched.

### Runtime retention

The adapter's process-local LRU holds 16 runtimes, keyed by resolved workspace, profile
and worker identity. Each runtime caps auto-approved previews at 64 entries or 128 MiB
encoded artifacts. Capacity fails closed; previews are not silently evicted within a
runtime to make room. The **runtime LRU can evict a whole runtime**, losing its
auto-approved handles even while the server process remains alive.

Restart also loses those handles. The registered tool automatically discards completed
auto-approved previews with no canonical changes: their response has `preview_retained=false`
and cannot be promoted. Mutating or approval-required previews remain retained. A host
which needs read-only promotion can register `retain_read_only_previews=True` and own the
cleanup lifecycle explicitly. This releases preview retention, not general blob-store
garbage collection; do not treat the cache as a durable queue.

Pending approval artifacts are durable. MCP does not expose approval minting: a trusted
Python/Rust service must authenticate the reviewer and call `approve`. Model-authored
output must not authorize itself. Denied work cannot be approved.

## VSH functions and CodeMode

In the current checkout, programs receive `vsh_read`, `vsh_write`, `vsh_list`,
`vsh_mkdir`, `vsh_remove`, `vsh_move`, `vsh_copy`, `vsh_glob`, `vsh_search` and
`vsh_patch`. They are guest callables, not extra MCP tools, host SDK methods, nested
transactions or access to the host filesystem. They share the overlay with `pathlib`.

CodeMode can append trusted project guidance from `VSH_CODEMODE_INSTRUCTIONS_FILE`
and `VSH_CODEMODE_INSTRUCTIONS`. File content precedes inline content; built-in guidance
remains first. Instructions are advice, not enforcement of roots, profiles or budgets.

## Run the protocol example

```bash
uv run --no-sync python examples/native/mcp_workflow.py
```

The source-checkout recipe uses FastMCP's real client and in-process MCP transport,
lists exactly one tool and performs preview/review/promotion in one server lifetime.
It verifies the resulting fixture file and needs no model credentials. It does not
claim to benchmark external MCP transports or agent token cost.

Continue with [agent deployment](agents.md), the [function reference](monty-tools.md)
and [efficient lifecycle management](../guides/efficient-usage.md).
