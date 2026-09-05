# MCP server

VSH exposes one normal MCP tool, `vsh_run`. Its `code` argument is a complete Monty
program: one snapshot, one active overlay, one canonical change set and one policy
decision. The Python adapter constructs a native request and projects the receipt;
it does not implement a separate filesystem simulator.

## Install and launch

```bash
python -m pip install 'vsh-python[mcp]==0.4.0'
vsh serve
```

Transport is stdio. `vsh-codemode` exposes the same one-tool surface with built-in
workflow instructions and the `vsh_run_transaction` prompt:

```bash
vsh-codemode
```

The ten in-program VSH functions and the existing `pathlib` surface are included in
VSH 0.4.0. See [source installation](../development.md) when developing from a checkout.

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
Client configuration formats vary; `cwd` selects the default workspace, **not an
authorization allowlist**. The raw tool still accepts `workspace_root`, policy and
budget arguments. Constrain those in a trusted wrapper for an untrusted client.

## Request contract

```text
vsh_run(
    code: str | None = None,
    *,
    transaction: str | None = None,
    workspace_root: str | None = None,
    intent: str | None = None,
    mode: "preview" | "auto" = "preview",
    policy: "balanced" | "strict" | "paranoid" = "balanced",
    detail: "compact" | "full" = "compact",
    budget: BudgetOverrides | None = None,
) -> dict[str, object]
```

For new work pass `code`. For promotion pass `transaction`, no code, and `mode="auto"`.
Resolve the same workspace/profile/worker identity as the preview. Promotion does not
rerun source or create a different detail/budget configuration for the existing artifact.
The budget keys match the [Python execution budget](../python/api.md#executionbudget).

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
| Top-level guest output | `result_repr`, `result_truncated`, `stdout`, `stdout_truncated` |
| `execution` | `os_calls`, `read_bytes`, `write_bytes`, `directory_entries`, `output_bytes`, `denied_accesses`, `result_bytes` |
| `commit` | `committed`, `operations`, `verified_paths`, `cleanup_pending` |
| `timings_ns` | `snapshot`, `execute`, `diff`, `policy`, `bind_and_store`, `commit`, `total` |

MCP returns `result_repr`, not the Python SDK's arbitrary typed `result`. Do not `eval`
that representation. `diff` is not a textual diff; request bounded before/after content
when reviewing a transformation.

The adapter retains **65,536 Python characters plus an ellipsis** independently for
result representation and stdout. This is not 64 KiB of UTF-8, a whole-envelope cap or
a token limit. JSON escaping and full change lists add transport bytes. Truncation
happens after constructing the representation; return small results in the first place.

## Lifetime and retention limits

The adapter's process-local LRU holds 16 runtimes, keyed by resolved workspace, profile
and worker identity. Each runtime caps auto-approved previews at 64 entries or 128 MiB
encoded artifacts. Capacity fails closed; previews are not silently evicted within a
runtime to make room. The **runtime LRU can evict a whole runtime**, losing its
auto-approved handles even while the server process remains alive.

Restart also loses those handles. Read-only previews consume capacity too, and the raw
MCP surface has no discard tool. For high-rate analysis, use an SDK-owned service that
can discard completed previews; do not treat the raw cache as a durable queue.

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
