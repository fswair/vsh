# Pydantic AI capability

`VshCapability` gives a Pydantic AI agent a capability-rooted filesystem without
giving the model a host shell, a host filesystem mount, subprocess access, or a
second simulation engine. Eleven agent tools enter the same Rust-owned VSH runtime
used by the Python and Rust SDKs.

Use this page as the integration reference. For a guided build, continue with either
[deterministic review](../tutorials/pydantic-ai-deterministic.md) or the
[evidence-first LLM judge](../tutorials/pydantic-ai-judge.md).

## Choose the control layer

| Requirement | Configuration | Who may authorize commit? |
|---|---|---|
| Native policy is enough | `VshCapability(workspace, policy=...)` | Native VSH policy |
| Exact application rules | `hook_handler=my_handler` | Your deterministic Python code |
| Semantic review of complete evidence | `hook_handler=judge.hook_handler` | Explicitly configured `CommitJudge` |
| Human or external workflow | Return `HookDecision.review(...)` | Your later authenticated approval path |

A hook does not replace native policy. Hard-denied operations never become approvable,
and every commit still passes capability, transaction, single-use, dependency, and
stale-state checks.

## Install and attach

Install the optional integration. The published distribution is named `vsh-python`;
imports remain under `vsh`.

```bash
uv add "vsh-python[pydantic-ai]"
```

The extra pins `pydantic-ai-slim==2.40.0`. Install the provider package required by
your model separately and configure its credentials through that provider. Importing
base `vsh` does not load Pydantic AI; only `vsh.pydantic_ai` requires the extra.

This complete application gives one agent a balanced-policy workspace:

```python
import asyncio
import os

from pydantic_ai import Agent
from vsh.pydantic_ai import VshCapability


async def main() -> None:
    filesystem = VshCapability(
        "/srv/my-project",
        policy="balanced",
    )
    agent = Agent(
        os.environ["PYDANTIC_AI_MODEL"],
        instructions=(
            "Use only the supplied VSH tools for workspace files. Make the smallest "
            "change that satisfies the request. If VSH returns pending_approval, report "
            "its transaction and feedback instead of claiming success."
        ),
        capabilities=[filesystem],
    )
    result = await agent.run("Set timeout_seconds to 30 in config/service.toml.")
    print(result.output)


asyncio.run(main())
```

`VshCapability(workspace, ...)` constructs and owns its runtime. There is deliberately
no `VshCapability.open(...)`. `Runtime.open(...)` remains the native SDK entry point
when an application is not using the Pydantic AI capability.

## Execution model

```text
Pydantic AI agent
       │ tool call
       ▼
VshCapability ── one RunRequest ──► native VSH runtime
                                      │
                                      ├─ immutable base snapshot
                                      ├─ Monty + virtual filesystem
                                      ├─ canonical diff + policy
                                      ├─ optional commit hook
                                      └─ revalidate + commit, or keep pending
       ▲
       └──────────── VshToolResult ────────────────────────────────
```

Each ordinary tool is one transaction. VSH starts from a fresh host snapshot, runs the
operation against the virtual filesystem, freezes the resulting evidence, evaluates
policy, and commits only when authorized. Tool registration is sequential, preventing
parallel calls from the same model turn from creating avoidable same-workspace races.
Separate agents or capabilities can still race; native stale checks protect commit.

## Constructor reference

```text
VshCapability(
    workspace,
    *,
    data_directory=None,
    policy="balanced",
    worker_path=None,
    hook_handler=None,
    hook_scope=HookScope.REVIEW_REQUIRED,
    hook_id="vsh.pydantic-ai",
    review_content_bytes=0,
    id="vsh",
    defer_loading=False,
)
```

| Parameter | Meaning |
|---|---|
| `workspace` | Existing host directory exposed inside Monty as `/workspace` |
| `data_directory` | Trusted VSH state location; it must not overlap the workspace |
| `policy` | Native transaction profile, normally `balanced` or `strict` |
| `worker_path` | Explicit matching Monty worker, mainly useful from a source checkout |
| `hook_handler` | Sync or async application-owned `RequestEvent -> HookDecision` handler |
| `hook_scope` | Review only policy-pending work, or every non-denied request |
| `hook_id` | Stable identity bound into hook evidence and approvals |
| `review_content_bytes` | Native byte budget for immutable before/after/read content |
| `id` | Pydantic AI capability and toolset identifier |
| `defer_loading` | Defer toolset loading through Pydantic AI's capability mechanism |

Use a new `hook_id` when the meaning of your review rules changes. This prevents audit
records from making two materially different review policies look identical.

## Tool reference

All paths are capability-rooted Monty paths. Prefer `/workspace/...` in prompts and
tool calls; never put a host absolute path into model instructions.

| Tool | Signature | Use it for |
|---|---|---|
| `vsh_read` | `(path)` | One UTF-8 file |
| `vsh_write` | `(path, data, append=False)` | Create, replace, or append text |
| `vsh_list` | `(path="/workspace")` | One directory listing |
| `vsh_mkdir` | `(path, parents=True, exist_ok=True)` | Directory creation |
| `vsh_remove` | `(path, recursive=False, missing_ok=False)` | File or bounded tree removal |
| `vsh_move` | `(source, destination)` | One virtual rename/move |
| `vsh_copy` | `(source, destination, recursive=False, overwrite=False)` | File or bounded tree copy |
| `vsh_glob` | `(pattern, path="/workspace", max_results=1000)` | Bounded path matching |
| `vsh_search` | `(query, path="/workspace", case_sensitive=True, max_results=100)` | Bounded literal text search |
| `vsh_patch` | `(path, old, new, count=1)` | Exact text replacement |
| `vsh_run` | `(code, intent)` | Dependent work in one atomic transaction |

### One tool or one compound transaction?

Use ordinary tools when operations are independent. This is clear and gives the agent
small structured results:

```python
await filesystem.vsh_read("/workspace/config/service.toml")
await filesystem.vsh_patch(
    "/workspace/config/service.toml",
    "timeout_seconds = 10",
    "timeout_seconds = 30",
)
```

Use `vsh_run` when a later step depends on an earlier virtual result, or when several
changes must be reviewed and committed together:

```python
result = await filesystem.vsh_run(
    """
config = vsh_read('/workspace/config/service.toml')
if 'require_auth = true' not in config:
    raise ValueError('authentication invariant is missing')
vsh_patch(
    '/workspace/config/service.toml',
    'timeout_seconds = 10',
    'timeout_seconds = 30',
)
vsh_write('/workspace/config/change-note.txt', 'timeout: 10 -> 30\n')
""",
    intent="Raise the service timeout while preserving authentication.",
)
```

Code passed to `vsh_run` is Python syntax executed by Monty, but it can call only the
ten listed `vsh_*` filesystem functions. `read_file` and `write_file` do not exist.
Pass normal Python arguments—do not pass one JSON object as a positional argument.
The `intent` is useful context for policy and review, never authority or proof.

## Agent-visible results

Every tool returns a `VshToolResult`:

```text
transaction: str
state: str
result: JSON-compatible value
changed_paths: int
hook_verdict: str | None
feedback: str | None
requires_review: bool
```

| State | Host effect | `result` | What the agent should do |
|---|---|---|---|
| `committed` | Exact transaction committed and verified | Available | Report completion |
| `pending_approval` | No proposed mutation committed | `None` | Report transaction and feedback; revise or hand off |
| `rejected` | No proposed mutation committed | `None` | Report rejection; do not retry unchanged work |
| `denied` | Native policy/capability refused execution | `None` | Explain the denial; change the request or configuration |

`result.requires_review` is true only for `pending_approval`. Monty paths become
strings, tuples become arrays, mappings are normalized recursively, and bytes use
`{"encoding": "base64", "data": "..."}`.

Guest results are withheld until commit when a hook is attached. Review feedback can
therefore describe a simulated read or write without releasing its guest return value.
This does not erase reads already performed inside the isolated virtual execution.

Give the main agent an explicit response rule:

```python
agent = Agent(
    model,
    instructions=(
        "When a VSH result is pending_approval, do not say the task is complete. "
        "Explain the feedback, preserve the transaction ID, and propose the smallest "
        "corrected transaction."
    ),
    capabilities=[filesystem],
)
```

## Deterministic hooks

Application code can decide from immutable native evidence:

```python
from vsh import HookDecision, HookScope, RequestEvent
from vsh.pydantic_ai import VshCapability


def review(event: RequestEvent) -> HookDecision:
    if not event.evidence_complete or event.evidence_truncated:
        return HookDecision.review("Complete evidence is required.")
    if event.deleted_paths:
        return HookDecision.review("Deletion requires a second reviewer.")
    if event.touched_paths > 4:
        return HookDecision.review("Split this into at most four changed paths.")
    return HookDecision.follow_policy()


filesystem = VshCapability(
    "/srv/my-project",
    policy="balanced",
    hook_handler=review,
    hook_scope=HookScope.ALL_REQUESTS,
    hook_id="bounded-change-review-v1",
)
```

`follow_policy()` preserves the native result: auto-approved work commits, while
policy-pending work stays pending. `approve(...)` authorizes the exact complete
transaction; `review(...)` keeps it pending with feedback; `reject(...)` makes it
terminal. See the [deterministic tutorial](../tutorials/pydantic-ai-deterministic.md)
for an exact-content allowlist.

## LLM judge hooks

An LLM judge is opt-in. It is an explicit hook object, not an implicit property of the
capability:

```python
from vsh.pydantic_ai import CommitJudge, VshCapability

judge = CommitJudge(
    model=judge_model,
    review_instructions=(
        "Authentication must remain enabled. Approve a timeout-only edit only when "
        "the complete after-content proves require_auth = true."
    ),
    content_filter=lambda path: path == "config/service.toml",
)
filesystem = VshCapability(
    "/srv/my-project",
    policy="strict",
    hook_handler=judge.hook_handler,
    hook_id="service-config-judge-v1",
    review_content_bytes=64 * 1024,
)
```

`review_instructions` extends VSH's fixed evidence-first instruction set; it does not
replace it. `content_filter` is separate from native capture: both the native byte
budget and explicit host permission must cover every required path before a model call
is made. Continue with the [judge tutorial](../tutorials/pydantic-ai-judge.md) and
[judge reference](../python/commit-judge.md).

## Runtime, concurrency, and cleanup

- Keep one capability per workspace/configuration instead of constructing one per tool
  call; it owns runtime stores and a supervised worker pool.
- Tool calls contributed by one capability are sequential. Bound concurrent agent runs
  at the application level as well.
- Independent capabilities targeting the same workspace can preview concurrently, but
  commits revalidate host identity and dependencies and may raise a stale conflict.
- The runtime's trusted data directory must remain outside the untrusted workspace.
- From a source checkout, build the matching worker with
  `cargo build --release --locked -p vsh-monty-worker` and pass `worker_path` if package
  discovery is unavailable.

## Troubleshooting map

| Symptom | Likely cause | Action |
|---|---|---|
| Tool says `pending_approval` | Policy or hook withheld approval | Surface `feedback`; do not claim completion |
| Judge never runs | Default scope ignores auto-approved work | Use strict policy or deliberately select `ALL_REQUESTS` |
| Judge reports incomplete evidence | Content capture/filter/budget is insufficient | Check `review_content_bytes`, `content_filter`, file type, and size |
| `vsh_run` reports an unknown function | Model invented a non-VSH helper | Use the exact `vsh_*` names and Python call syntax above |
| Commit becomes stale | Host changed after preview | Produce and review a new transaction; never reuse the old approval |
| Worker cannot start | Matching packaged/source worker is unavailable | Build it and set `worker_path` explicitly |

For lower-level lifecycle details, see [commit hooks](../python/hooks.md),
[policies and budgets](../guides/policies-and-budgets.md), and the
[security model](../security.md).
