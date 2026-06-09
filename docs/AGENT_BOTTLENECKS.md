# vsh Agent Context and Speed Bottlenecks

This report summarizes the live comparison in
`playground/reports/agent-context-20260609-094621/comparison.md`.

Both agents completed the same scenario successfully:

| Metric | vsh CodeMode | native structured FS tools |
|---|---:|---:|
| Wall time | 57.2 s | 8.4 s |
| Input tokens | 236,587 | 3,479 |
| Total tokens | 239,101 | 3,758 |
| Serialized history | 186,908 bytes | 12,936 bytes |
| Tool calls | 33 | 5 |

The bottleneck is not local filesystem execution. The issue is the agent-facing
protocol: `search/get_schema -> snapshot -> simulate -> approve -> execute`
turns a small workflow into many model round trips. `vsh_sandbox` reduces some
simulation calls, but execution still requires separate approval and execution
tool calls.

The fix is to keep the validation-first internals, but expose a shorter agent
path:

- `apply`: simulate, approve, and execute one command, returning a compact receipt.
- `apply_batch`: run multiple validated steps against a reused snapshot.
- Compact receipts by default, full simulation/execution payloads only when asked.
- Reuse the runtime snapshot after execution instead of forcing another full
  workspace snapshot.

Success targets for the comparison workflow:

- vsh stays correct.
- vsh tool calls fall below 8.
- vsh model requests fall below 10.
- vsh input tokens fall below 30k for the benchmark.
- vsh wall time falls below 15 seconds for the benchmark.
