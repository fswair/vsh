# Tutorial: deterministic Pydantic AI review

This tutorial builds a Pydantic AI filesystem capability whose commit decision is
made entirely by local Python code. The example permits exactly one final
`service.toml` value and sends every other proposal back for review.

Choose this design when the rule can be expressed precisely. It is faster, cheaper,
and easier to test than an LLM judge. Add a judge only for decisions that genuinely
need semantic interpretation.

## What you will build

```text
agent tool call
      │
      ▼
VSH simulation ──► exact canonical diff + immutable content
                                      │
                                      ▼
                              local allowlist handler
                                │              │
                             approve         review
                                │              │
                           native commit   agent feedback
```

The allowlist approves only this complete after-content:

```toml
timeout_seconds = 30
require_auth = true
```

Changing authentication, adding another path, deleting the file, or producing
incomplete content evidence remains pending.

## 1. Install the integration

```bash
uv add "vsh-python[pydantic-ai]"
```

Create the workspace before constructing the capability. In an installed wheel the
matching worker is discovered automatically. A source checkout may need an explicit
`worker_path`; see the [capability reference](../integrations/pydantic-ai.md).

## 2. Write the smallest complete rule

The handler maps the after-node's immutable blob identifier back to the captured bytes.
It does not trust intent or re-read the current host file.

```python
from vsh import HookDecision, RequestEvent

EXPECTED_SERVICE = b"timeout_seconds = 30\nrequire_auth = true\n"


def approve_exact_service_config(event: RequestEvent) -> HookDecision:
    if not event.evidence_complete or event.evidence_truncated:
        return HookDecision.review("Complete canonical evidence is required.")
    if not event.content_complete:
        return HookDecision.review("Complete before/after content is required.")
    if len(event.canonical_diff) != 1:
        return HookDecision.review("Change only config/service.toml in this transaction.")

    change = event.canonical_diff[0]
    if change.path != "config/service.toml" or change.kind != "modify":
        return HookDecision.review("Only an in-place service config update is allowed.")
    if change.after is None or change.after.content is None:
        return HookDecision.review("The final service config content is unavailable.")

    content_by_blob = {item.blob: item.bytes for item in event.contents}
    if content_by_blob.get(change.after.content) != EXPECTED_SERVICE:
        return HookDecision.review(
            "Set timeout_seconds to 30 and preserve require_auth = true exactly."
        )
    return HookDecision.approve("The complete after-content matches the local allowlist.")
```

Why each check matters:

1. `evidence_complete` protects the structural event and canonical diff.
2. `content_complete` proves no required before/after/read content was omitted.
3. One exact relative path prevents a safe edit from carrying an unrelated side effect.
4. `kind == "modify"` excludes creation, deletion, and metadata-only substitution.
5. The blob lookup binds the decision to immutable review content, not a later host read.
6. The byte equality makes the approved state explicit and testable.

This rule is deliberately narrow. A broader parser-based rule is reasonable when
formatting may vary, but parse the complete bytes and reject duplicate keys, invalid
syntax, and fields outside your allowlist. Substring checks are not sufficient for
security-sensitive configuration.

## 3. Construct the capability

Use strict policy so every mutation reaches the handler. Allocate enough native review
content for the complete before and after file.

```python
from vsh import HookScope
from vsh.pydantic_ai import VshCapability

filesystem = VshCapability(
    workspace,
    policy="strict",
    hook_handler=approve_exact_service_config,
    hook_scope=HookScope.REVIEW_REQUIRED,
    hook_id="exact-service-config-v1",
    review_content_bytes=4 * 1024,
)
```

`REVIEW_REQUIRED` is the default and is shown explicitly here. Native hard-denies do
not invoke the handler. Under strict policy, eligible writes become pending first;
the handler may then approve only the exact prepared transaction.

## 4. Prove both branches without an LLM

Test the capability directly before attaching it to an agent:

```python
safe = await filesystem.vsh_write(
    "/workspace/config/service.toml",
    "timeout_seconds = 30\nrequire_auth = true\n",
)
assert safe.state == "committed"
assert safe.hook_verdict == "approve"

unsafe = await filesystem.vsh_write(
    "/workspace/config/service.toml",
    "timeout_seconds = 30\nrequire_auth = false\n",
)
assert unsafe.state == "pending_approval"
assert unsafe.hook_verdict == "review"
assert unsafe.result is None
assert "preserve require_auth" in (unsafe.feedback or "")
```

Also assert the host file after each case. A model-facing status is useful, but the
actual safety property is that only the approved bytes reached the host.

```python
assert (workspace / "config/service.toml").read_bytes() == EXPECTED_SERVICE
```

## 5. Attach a real Pydantic AI agent

The same tested capability can now be supplied to an agent. Keep commit rules in the
handler and task behavior in the agent instructions.

```python
import os

from pydantic_ai import Agent

agent = Agent(
    os.environ["PYDANTIC_AI_MODEL"],
    instructions=(
        "Use the VSH tools for every workspace operation. Preserve require_auth=true. "
        "If a result is pending_approval, report its transaction and exact feedback; "
        "do not claim the host changed."
    ),
    capabilities=[filesystem],
)

result = await agent.run(
    "Read config/service.toml and set timeout_seconds to 30 without changing anything else."
)
print(result.output)
```

Expected safe flow:

1. The agent reads or patches `/workspace/config/service.toml`.
2. VSH simulates the call and freezes the canonical diff and content.
3. The handler matches the exact final bytes and returns `approve`.
4. Native stale and capability checks pass, then VSH commits.
5. The agent receives `state="committed"` and the guest result.

Expected unsafe flow:

1. The agent proposes `require_auth = false`, deletion, or an unrelated second file.
2. The handler returns `review` with a concrete correction.
3. The agent receives `state="pending_approval"`, `result=None`, and feedback.
4. No proposed host mutation is applied. A corrected proposal creates a new transaction.

## Pattern: deterministic guardrail, native policy otherwise

Not every handler should directly approve. This second pattern only vetoes broad or
destructive work and delegates everything else back to policy:

```python
from vsh import HookDecision, HookScope, RequestEvent
from vsh.pydantic_ai import VshCapability


def bounded_change_gate(event: RequestEvent) -> HookDecision:
    if not event.evidence_complete or event.evidence_truncated:
        return HookDecision.review("Complete evidence is required.")
    if event.deleted_paths or event.symlink_changes or event.executable_changes:
        return HookDecision.review("Deletion, symlink, and executable changes need review.")
    if event.touched_paths > 5 or event.changed_bytes > 32 * 1024:
        return HookDecision.review("Split this proposal into a smaller transaction.")
    return HookDecision.follow_policy()


filesystem = VshCapability(
    workspace,
    policy="balanced",
    hook_handler=bounded_change_gate,
    hook_scope=HookScope.ALL_REQUESTS,
    hook_id="bounded-change-gate-v1",
)
```

`ALL_REQUESTS` makes the hook observe read-only and policy-auto-approved requests too.
Use it only when that visibility is required. `follow_policy()` never upgrades a
pending decision; it preserves the native baseline.

## Production checklist

- Start with the narrowest path, change-kind, size, and content rule you can express.
- Check completeness before approving; never infer unavailable bytes from a digest.
- Treat `event.intent` as untrusted context rather than an approval signal.
- Keep `hook_id` stable for one rule version and change it when semantics change.
- Test safe, missing-field, deletion, additional-path, binary, oversized, and stale cases.
- Return actionable `review` feedback for correctable work; reserve `reject` for terminal
  decisions.
- Keep external network calls out of a deterministic handler. If remote coordination is
  required, use an async hook and explicit timeout/capacity controls.
- Verify the host state in tests, not only the `VshToolResult` or agent prose.

Next, read the [hook lifecycle reference](../python/hooks.md). If your rule cannot be
made deterministic, continue with the [LLM judge tutorial](pydantic-ai-judge.md).
