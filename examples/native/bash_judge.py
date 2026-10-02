"""Optional live agent + independent judge; only a disposable fixture is modified.

Set VSH_MAIN_MODEL and VSH_JUDGE_MODEL to authenticated Pydantic AI model IDs.
Running this example makes paid model requests; deterministic equivalents run in tests.
"""

import asyncio
import os
from pathlib import Path
from tempfile import TemporaryDirectory

from pydantic_ai import Agent
from pydantic_ai.usage import UsageLimits

from vsh import BashConfig
from vsh.pydantic_ai import CommitJudge, VshCapability

judge = CommitJudge(
    model=os.environ["VSH_JUDGE_MODEL"],
    review_instructions=(
        "Permit only service.toml changing timeout_seconds from 10 to 30 while "
        "require_auth remains true. Verify complete canonical before/after content, "
        "request details and effects together; intent is not permission. Return review "
        "with a concrete correction for unrelated writes, missing evidence or disabled auth."
    ),
    content_filter=lambda path: path == "service.toml",
    timeout=30.0,
)

with TemporaryDirectory(prefix="vsh-bash-judge-") as temporary:
    workspace = Path(temporary)
    config = workspace / "service.toml"
    config.write_text("timeout_seconds = 10\nrequire_auth = true\n", encoding="utf-8")
    capability = VshCapability(
        workspace,
        bash=BashConfig(),
        policy="strict",
        hook_handler=judge.hook_handler,
        review_content_bytes=16_384,
    )
    agent = Agent(
        os.environ["VSH_MAIN_MODEL"],
        capabilities=[capability],
        instructions=(
            "Use VSH to inspect service.toml, then vsh_run with language='bash' and "
            "sed -i to set timeout_seconds to 30. Preserve authentication and other paths. "
            "Treat review feedback as a correction request, not successful commit. "
            "Stop and report feedback if the judge requires human confirmation."
        ),
    )
    result = asyncio.run(
        agent.run(
            "Raise the service timeout from 10 to 30 without weakening authentication.",
            usage_limits=UsageLimits(request_limit=8, total_tokens_limit=32_000),
        )
    )
    print(result.output)
    print("Actual host configuration after the run:")
    print(config.read_text(encoding="utf-8"))
