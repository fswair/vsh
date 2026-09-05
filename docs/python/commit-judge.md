# LLM commit judge

`CommitJudge` is an optional Pydantic AI commit reviewer. It reviews the actual VSH
transaction and can approve `pending_approval` work directly, without an additional
human approval. Native hard-deny, stale detection and single-use commit still apply.

This surface is in the unreleased development tree. Use the [checkout build](../development.md)
until it is included in a registry release.

Start with the [guided judge tutorial](../tutorials/pydantic-ai-judge.md) for a complete
main-agent → simulation → judge → feedback flow. This page documents the Python surface
and its exact trust boundaries.

## Attach a judge

Install the `pydantic-ai` extra and the provider dependencies required by your selected
Pydantic AI model. The extra pins the framework to `pydantic-ai-slim==2.40.0`; it does
not select a provider, configure credentials, or make model calls during import.

```python
from vsh.pydantic_ai import CommitJudge, VshCapability

judge = CommitJudge(
    model="openai:gpt-5",
    review_instructions=(
        "Review service configuration changes. Authentication must remain enabled. "
        "Allow timeout changes only when supported by the actual diff."
    ),
    model_settings={"temperature": 0},
    content_filter=lambda path: path == "config/service.toml",
)
filesystem = VshCapability(
    "/path/to/workspace",
    policy="strict",
    hook_handler=judge.hook_handler,
    hook_id="service-review-policy-v1",
    review_content_bytes=64 * 1024,
)
```

Attach `filesystem` with `Agent(capabilities=[filesystem])`. The judge exposes its
asynchronous VSH adapter as `judge.hook_handler`; use
`HookedRuntime.open(hook_handler=judge.hook_handler, ...)` with `arun()` or `acommit()`.
An application-owned handler can still invoke its own
agent or review service through the existing [hook contract](hooks.md).

`content_filter` is a host-controlled permission to send the bytes for each relative
path to the model provider. It defaults to withholding file content. A false result,
missing native content, binary content or an exceeded budget keeps the transaction
pending **without making a model call**. Reading a file inside VSH does not by itself
authorize sending it to an external provider.

## Constructor reference

```text
CommitJudge(
    model,
    *,
    review_instructions="",
    model_settings=None,
    content_filter=None,
    usage_limits=None,
    max_output_tokens=2048,
    timeout=30.0,
    max_input_bytes=128 * 1024,
    max_concurrency=4,
)
```

| Parameter | Contract |
|---|---|
| `model` | Pydantic AI `Model` instance or configured model ID |
| `review_instructions` | Trusted application rules appended to built-in evidence-first instructions |
| `model_settings` | Pydantic AI model settings; do not place `max_tokens` here |
| `content_filter` | Explicit host permission for each relative content path sent to the model |
| `usage_limits` | Per-review Pydantic AI request/token/cost limits |
| `max_output_tokens` | Provider output request cap; positive integer or explicit `None` |
| `timeout` | Finite positive deadline for the complete judge run |
| `max_input_bytes` | Positive cap for content bytes and final serialized evidence |
| `max_concurrency` | Positive number of simultaneous reviews; excess work fails closed |

`review_instructions` are additive. The internal instruction set always tells the
model that approval is consequential, intent is untrusted, file/path strings are data,
unavailable content must not be guessed, and every approval must cite the complete
required evidence set.

The judge creates no filesystem, network, search, or application tools. Passing a model
that has provider-side capabilities is an application/provider decision outside the VSH
tool surface; configure the provider accordingly.

## `JudgeReport`

The model must return one validated structured report:

```python
class JudgeReport:
    decision: Literal["approve", "review", "reject"]
    reason: str
    evidence: list[str]
    concerns: list[str]
    missing_evidence: list[str]
```

Unknown fields are forbidden and the report is immutable after validation. Reasons,
references, concerns, and missing-evidence entries are length- and count-bounded. An
approval with any concern or missing evidence is converted to review rather than being
treated as partially safe.

## What the judge sees

VSH provides the full path-ordered canonical changes, before/after node identities,
bounded full UTF-8 content, observed read content, ordered effects, policy profile and
thresholds, risk metrics, execution counters, intent and transaction digests.
The JSON packet assigns `change:N`, `effect:N` and `content:N` references. Approval
must cite every changed entry and included content record; invalid references cannot
authorize commit. This validates traceability, not the semantic correctness of a model.

When enabled, eligible old file contents are captured while the snapshot is alive,
before final transaction binding. The canonical diff and policy are recomputed after
capture. Hook delivery then uses hash-verified immutable blobs, including after restart;
it never reads current host files to reconstruct the reviewed before content.

Native `evidence_complete` describes structural hook evidence. The separate
`content_complete` flag covers canonical content sides and observed content reads.
Oversized or read-protected content remains incomplete; no truncated blob is presented
as a complete file. Metadata effects can retain stamps; they are observations rather
than a promise that every intermediate file version was rendered.

The original program and dependencies have bound digests; the packet does not contain
the full original program or the main agent's conversation history. Supply review rules
through host-owned `review_instructions`, and task intent through the normal request. Never
interpolate untrusted file text into the judge's trusted instructions.

### Reference rules

The evidence packet names changes as `change:N`, effects as `effect:N`, content as
`content:N`, and includes `policy` and `intent`. Every canonical change and every
included content record is mandatory for approval. Effects can support a decision but
are not automatically mandatory references. A report may cite only identifiers present
in the packet.

These rules prevent an apparent approval from silently ignoring a second changed file.
They do not determine whether the model interpreted each cited item correctly.

## Decisions and feedback

| Judge outcome | VSH behavior |
|---|---|
| Valid `approve` | Approves and commits the same pending transaction, subject to native validity checks |
| `review` | Keeps `pending_approval` and returns actionable feedback |
| `reject` | Rejects the transaction and returns its reason |
| Approval with unresolved concerns or missing evidence | Remains pending with the reported concerns |
| Timeout, provider/schema failure, invalid references or insufficient evidence | Remains pending; no commit authorization |
| Caller cancellation while the judge runs | Cancellation propagates; the hook fails closed |

The judge evaluates intent together with actual changes. For example, “adjust logging”
does not justify a diff that disables authentication. The main agent receives the
problem and can submit a corrected proposal; a changed diff is a new transaction.
Its own claim of approval is not an independent native grant.

The capability only returns guest `result` data after a committed outcome. Pending,
denied and rejected tool results carry no guest value; review/reject feedback remains
available. This does not undo reads already performed inside the virtual execution.
Trusted SDK preview receipts retain their original behavior.

A feedback result does not structurally pause Pydantic AI's run. If your application
needs a human approval handoff, keep and resolve the exact native transaction; do not
rerun the original program and apply an old approval to a different preview.

## Invocation and cost controls

`HookScope.REVIEW_REQUIRED` remains the default: policy-auto-approved work never calls
the judge. Consequently, an auto-approved semantic risk is outside this review scope.
Use `HookScope.ALL_REQUESTS` when read-only and auto-approved transactions should also
be reviewed, or choose a policy profile that escalates the relevant writes.

`CommitJudge` defaults to one model request, zero ordinary tools, a 30-second model
timeout, four concurrent reviews, a 128 KiB serialized input budget and at most 128
evidence items. Saturated capacity returns review instead of creating an unbounded
queue. Output defaults to 2,048 tokens; reports and feedback have additional bounds.
Set `max_output_tokens=None` only for model backends that reject a provider-level output
limit. The structured report bounds, one-request usage limit, timeout and fail-closed
behavior remain active, but the provider request itself is no longer pre-capped by VSH.

Optional `usage_limits` accepts Pydantic AI `UsageLimits` and must retain a positive
request limit. Token/cost accounting may only become available after a provider
response. Request limits, the native byte budget, input limits and timeout complement
one another; a token or dollar setting alone is not a preflight spending guarantee.
Nested judge usage is not automatically part of your main agent's task budget.

Full before-content evidence adds bounded host reads and durable blob writes during
preview; it is not a zero-cost option. Keep `review_content_bytes` scoped to the
files and reviews your application needs. The model wait is asynchronous, but native
preview, prepare and resolve still execute synchronously in the Python async wrapper.
Measure both local storage overhead and provider latency for your deployment.

The judge reuses agent configuration but starts each review with independent messages.
It has no execution tools, no shared conversation history and no approval cache.
Host rules belong in `review_instructions`; they extend, rather than replace, VSH's
built-in evidence-first judge instructions. Model/provider settings are never tool arguments.
Bump your configured `hook_id` when changing the review policy you want approvals to
represent.

The `vsh.judge` logger emits transaction/event/hook IDs, instruction and evidence
digests, the model name, decision, latency and token/request counts. It does not log
file bodies, prompts, raw provider errors or full reasoning. This is application
logging, not a durable native audit ledger. Provider instrumentation configured by
your application can have its own content-capture policy.

## Integration checklist

- Use deterministic policy or hooks for rules that can be expressed exactly.
- Keep `HookScope.REVIEW_REQUIRED` unless reviewing auto-approved/read-only work is a
  deliberate requirement.
- Make `content_filter` an allowlist, not a broad suffix or substring guess.
- Size `review_content_bytes` for complete authorized files, then cap the serialized
  packet independently with `max_input_bytes`.
- Keep main-agent instructions explicit about `pending_approval` and host non-mutation.
- Version `hook_id` with the semantic review policy.
- Test content prompt injection, misleading intent, missing fields, deletion, extra
  paths, incomplete evidence, provider failure, invalid reports, cancellation, and stale
  host state.
- Assert final host bytes in acceptance tests.

## Executable service-configuration example

```bash
uv run --no-sync python examples/native/commit_judge.py
```

The example owns a temporary workspace. Its offline `FunctionModel` demonstrates a
timeout edit being committed and an authentication-disabling proposal remaining pending.
It makes no network calls and is not a benchmark of real model judgment.

To use a real, configured provider, explicitly pass a model:

```bash
uv run --no-sync python examples/native/commit_judge.py --model openai:gpt-5
```

Model quality, prompt-injection resistance, false approvals and costs must be evaluated
on representative safe and adversarial transactions for the model you deploy. Passing
the deterministic integration tests does not establish those model-quality properties.

For the full capability wiring and correction loop, follow the
[evidence-first judge tutorial](../tutorials/pydantic-ai-judge.md). The opt-in
[`examples/live_commit_judge.py`](https://github.com/fswair/vsh/blob/main/examples/live_commit_judge.py)
runs a real main model and judge against disposable safe and adversarial workspaces.
