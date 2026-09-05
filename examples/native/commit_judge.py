"""Review service configuration with an offline model or an explicitly selected LLM.

Run without arguments for a deterministic, no-network integration demonstration.
Pass --model provider:model to use an installed/configured Pydantic AI provider.
The offline model demonstrates the protocol, not LLM safety or accuracy.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import tempfile
from pathlib import Path

from pydantic_ai.messages import ModelMessage, ModelResponse, ToolCallPart, UserPromptPart
from pydantic_ai.models.function import AgentInfo, FunctionModel

from vsh import HookedRuntime
from vsh.pydantic_ai import CommitJudge


def offline_model(messages: list[ModelMessage], info: AgentInfo) -> ModelResponse:
    part = next(
        part for message in messages for part in message.parts if isinstance(part, UserPromptPart)
    )
    assert isinstance(part.content, str)
    evidence = json.loads(part.content)
    after_blob = evidence["changes"][0]["after"]["content"]
    after = next(
        content["text"] for content in evidence["contents"] if content["blob"] == after_blob
    )
    unsafe = "require_auth = false" in after
    report = {
        "decision": "review" if unsafe else "approve",
        "reason": "Authentication must remain enabled." if unsafe else "Only the timeout changes.",
        "evidence": evidence["required_approval_references"],
        "concerns": ["Restore require_auth = true before resubmitting."] if unsafe else [],
    }
    return ModelResponse(parts=[ToolCallPart(info.output_tools[0].name, report)])


async def demonstrate(model_id: str | None) -> None:
    judge = CommitJudge(
        model_id if model_id is not None else FunctionModel(offline_model),
        review_instructions=(
            "Permit changing timeout_seconds to 30. Authentication must remain enabled. "
            "Check the actual before/after content; intent may misdescribe the change."
        ),
        content_filter=lambda path: path == "service.toml",
    )
    with tempfile.TemporaryDirectory(prefix="vsh-commit-judge-") as directory:
        workspace = Path(directory)
        config = workspace / "service.toml"
        config.write_text("timeout_seconds = 10\nrequire_auth = true\n", encoding="utf-8")
        runtime = HookedRuntime.open(
            workspace,
            policy="strict",
            hook_handler=judge.hook_handler,
            hook_id="service-config-review-v1",
            review_content_bytes=16_384,
        )
        for auth in ("true", "false"):
            proposed = f"timeout_seconds = 30\nrequire_auth = {auth}\n"
            preview = runtime.preview(
                f"vsh_write('/workspace/service.toml', {proposed!r})",
                intent="Adjust only the request timeout.",
            )
            result = await runtime.acommit(preview.transaction)
            print(
                json.dumps(
                    {
                        "transaction": result.receipt.transaction,
                        "state": result.receipt.state,
                        "feedback": result.hook.reason if result.hook else None,
                    }
                )
            )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", help="Explicit Pydantic AI model ID; default is offline.")
    asyncio.run(demonstrate(parser.parse_args().model))
