# VSH in an agent environment

Give the agent a language for proposing changes, while the trusted application retains
workspace authority and the decision to apply them. VSH supplies virtual execution,
deterministic policy, exact transaction binding and commit revalidation. It does not
authenticate users or determine whether an agent understood the task.

## Choose an integration boundary

| Integration | Good fit | Host responsibility |
|---|---|---|
| Python/Rust SDK wrapper | Managed services and high-rate workflows | Own runtime lifecycle, fixed roots/budgets, review and discard |
| `vsh serve` | An MCP host with its own workflow guidance | Restrict raw arguments and manage server lifetime |
| `vsh-codemode` | MCP clients consuming server instructions/prompts | Same controls; instructions are not policy enforcement |

The raw MCP tool exposes workspace, policy and budget choices. Do not assume its
working directory confines an untrusted caller. Run with appropriate operating-system
permissions and place an authorization layer around those arguments.

## Keep trusted choices out of the model schema

For example, construct this once in your application; expose only source submission
through your agent tool framework:

```python
from vsh import ExecutionBudget, ReceiptDetail, Runtime

runtime = Runtime.open("/srv/workspaces/project-17", policy="strict")
budget = ExecutionBudget(
    max_duration_ms=250,
    max_memory_bytes=64 * 1024 * 1024,
    max_os_calls=1_000,
    max_read_bytes=8 * 1024 * 1024,
    max_write_bytes=4 * 1024 * 1024,
    max_output_bytes=16 * 1024,
    max_result_bytes=64 * 1024,
)

def propose(code: str):
    return runtime.preview(code, detail=ReceiptDetail.FULL, budget=budget)
```

This illustrates ownership, not a complete authenticated server. The application must
serialize a bounded receipt, enforce admission/rate limits, correlate user sessions,
clean up read-only auto-approved previews and protect review/commit endpoints. Do not
hand a `Runtime` object or an unrestricted `approve` method to the model.

## A reliable agent loop

1. Establish a trusted task and allowed workspace scope.
2. Ask for one bounded Monty program containing dependent discovery, edits and validation.
3. Preview it and distinguish execution errors from returned policy decisions.
4. Review exact paths, intended content, risks and output truncation.
5. Stop on denial; send pending work to an independent authenticated reviewer.
6. Commit only the exact accepted transaction, using the correct retained runtime.
7. Require verified commit evidence before claiming completion.

If input drift makes the artifact stale, re-plan and review against a fresh snapshot.
Never route a denied operation through a shell or another filesystem tool to bypass
VSH's result.

## Compose inside the active snapshot

The [VSH functions](monty-tools.md) make bounded
compound operations concise. `pathlib` and `vsh_*` see each other's writes. A second
`vsh_run` gets a fresh host snapshot, so do not spread dependent staging steps across
multiple previews expecting a persistent virtual session.

Use cap+1 discovery and expected-content assertions for migrations. Prefer one coherent
program over many external read/write calls, but split independently reviewable batches
when their combined evidence exceeds practical limits. The [cookbook](../python/examples.md)
demonstrates these patterns with actual fixture outcomes.

## Treat review evidence as untrusted data

`diff` is a digest, not a text diff. Full Python/MCP changes identify paths and kinds;
Rust additionally exposes node-state before/after identities. A review UI needs bounded
content evidence as well as scope. Do not accept `{'safe': True}` returned by the same
program as proof of safety.

File text, returned objects, stdout and source can contain prompt injection. Present
them as quoted data, separate them from trusted instructions, and bind the final review
to the transaction ID. The trusted service authenticates the reviewer before creating
a principal-bound, expiring grant.

## Manage runtime and process resources

Reuse one runtime per authorized workspace/configuration. Complete every auto-approved
preview lifecycle; read-only requests must be discarded after consumption. Use strict
durable artifacts for asynchronous human review, not process-local auto-preview caches.

The raw MCP LRU retains 16 runtimes and can lose handles on eviction. Its one-tool
surface has no discard action. SDK wrappers are better suited to continuous analysis
and long-lived services. Bound concurrent active requests independently of idle worker
pool capacity, and expect stale conflicts for concurrent work on one workspace.

Guest bytecode/heap limits are not whole-service deadlines or memory quotas. Monitor
parent plus worker processes and apply deployment-level limits. Separate external
network/build tasks into explicit trusted tools; VSH itself does not gain those
capabilities by being connected to an agent.

## Measure context and cost honestly

One external tool schema and a small result can reduce prompt/context volume. High-level
calls can reduce worker suspensions. These are different costs from snapshot I/O,
native allocations, MCP serialization and model billing. The local
[benchmarks](../performance.md) measure execution and resource behavior, not token prices
or model quality. Profile your own agent loop before claiming monetary savings.

The MCP cookbook is credential-free and tests protocol behavior. It is not an LLM
integration evaluation. Runnable examples under `examples/native/` use the current
native transaction API and disposable workspaces.
