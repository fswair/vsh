# Commit hooks

Commit hooks let trusted application logic inspect the exact output of a successful
VSH simulation before host mutation. They do not replace deterministic policy. A hard
policy denial never reaches a hook.

For a complete Pydantic AI application with a local evidence rule, follow the
[deterministic review tutorial](../tutorials/pydantic-ai-deterministic.md). For semantic
model review, use the separate [LLM judge tutorial](../tutorials/pydantic-ai-judge.md).

## Minimal Python handler

```python
from vsh import HookDecision, HookedRuntime, RequestEvent, RunMode, RunRequest


def review(event: RequestEvent) -> HookDecision:
    if event.deleted_paths:
        return HookDecision.review(
            "Deletion is present; confirm the exact canonical paths before approval."
        )
    return HookDecision.approve("Canonical diff contains no deletions.")


runtime = HookedRuntime.open(
    "/path/to/workspace",
    hook_handler=review,
)
receipt = runtime.run(
    RunRequest(
        "vsh_write('/workspace/result.txt', 'ready')",
        intent="write the generated result",
        mode=RunMode.AUTO,
    )
)
```

`HookScope.REVIEW_REQUIRED` is the default. The handler runs only when native policy
would return `pending_approval`. Select `HookScope.ALL_REQUESTS` to inspect successful
read-only and auto-approved simulations too. Denied access and denied mutation remain
unhookable.

## Evidence contract

`RequestEvent` is immutable and transaction-bound. It includes:

- transaction, event, hook, base snapshot, program, policy, runtime configuration,
  diff, read-set and write-set identities;
- the complete path-ordered canonical diff used by commit;
- ordered VFS/Monty effect observations and execution counters;
- deterministic risk metrics and sorted risk flags;
- bounded raw intent plus its transaction-bound digest;
- explicit `evidence_complete` and `evidence_truncated` markers.

Set `review_content_bytes` on `HookedRuntime.open` (or `Runtime.open` with a hook)
to opt into bounded immutable file content. `event.contents` contains `ReviewContent`
objects with `path`, `blob` and complete `bytes`; `event.content_complete` states
whether canonical before/after content and observed content reads were all included.
The default byte budget is zero. Native content-read permissions apply, and oversized
or unavailable content is never silently presented as complete. `policy_thresholds`
exposes the exact native threshold values to Python handlers.

Treat intent as untrusted context. Review the canonical diff, effects, dependencies,
policy result and intent together. `HookDecision.approve(...)` refuses legacy or
truncated evidence; a handler cannot approve data it did not receive completely.

## Decisions and lifecycle

| Decision | Result |
|---|---|
| `follow_policy()` | Auto-approved work commits; policy review stays pending |
| `approve(reason)` | Binds the hook principal to this exact transaction, then commits |
| `review(feedback)` | Keeps or moves the transaction to existing `pending_approval` |
| `reject(reason)` | Moves the transaction to terminal `rejected` |

Review feedback is returned in `CommitResolution.hook.reason`, while the lifecycle
state remains `pending_approval`. A main agent, human, or another authenticated
reviewer can then decide how to proceed.

Handler exceptions, cancellation, wrong return types, and sync calls that receive an
awaitable fail closed. An auto-approved transaction becomes `pending_approval`; host
files remain unchanged.

## Async handlers

Use `arun()` and `acommit()` when the handler can return an awaitable:

```python
async def review(event: RequestEvent) -> HookDecision:
    decision = await external_review(event)
    return HookDecision.approve(decision.reason)

runtime = HookedRuntime.open(workspace, hook_handler=review)
receipt = await runtime.arun(request)
```

VSH does not create an LLM implicitly. To construct an explicitly configured Pydantic AI
handler whose approval can commit pending work, see [LLM commit judge](commit-judge.md).

## Low-level prepare/resolve

Async frameworks and non-Python hosts can use the same two-phase boundary directly:

1. `Runtime.prepare_commit(transaction)` freezes or regenerates the exact event.
2. Run external code without holding VSH runtime locks.
3. `Runtime.resolve_commit(preparation, decision, now_ms)` revalidates state and event
   equality before applying the decision.
4. Call `Runtime.fail_hook(preparation)` when the external handler cannot finish.

A guarded native `Runtime.commit(...)` returns a typed hook-required error instead of
bypassing the configured hook.
