#!/usr/bin/env python3
"""pydantic-ai agent demo using the vsh capability."""

# ruff: noqa: E402

from __future__ import annotations as _annotations

import argparse
import os
import tempfile
from pathlib import Path

from dotenv import load_dotenv

load_dotenv()

from pydantic_ai.messages import ModelResponse, TextPart, ToolCallPart
from pydantic_ai.models.function import FunctionModel
from pydantic_ai.models.test import TestModel

from vsh.agent import create_vsh_agent


def _prepare_workspace(root: Path) -> None:
    src = root / "src"
    src.mkdir()
    (root / "README.md").write_text(
        "# Demo workspace\n\nHello from pydantic-ai + vsh.\n", encoding="utf-8"
    )
    (src / "app.py").write_text("print('hello')\n", encoding="utf-8")


def _build_scripted_model() -> FunctionModel:
    step = 0

    def next_step(_messages: object, _info: object) -> ModelResponse:
        nonlocal step
        step += 1
        if step == 1:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_search",
                        args={"query": "list"},
                        tool_call_id="demo_search",
                    )
                ]
            )
        if step == 2:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_snapshot_workspace",
                        args={},
                        tool_call_id="demo_snapshot",
                    )
                ]
            )
        if step == 3:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_simulate",
                        args={
                            "tool_name": "vsh_list",
                            "params": {"path": ".", "all": True},
                        },
                        tool_call_id="demo_simulate",
                    )
                ]
            )
        return ModelResponse(parts=[TextPart(content="vsh agent demo complete")])

    return FunctionModel(next_step)


def _resolve_live_model(cli_model: str | None) -> str:
    model = cli_model or os.environ.get("MODEL_NAME")
    if model is None:
        msg = "Set MODEL_NAME in .env or the environment, or pass --model."
        raise SystemExit(msg)
    return model


def _live_instructions() -> str:
    return (
        "You operate on a workspace using vsh structured commands. "
        "Follow this flow unless the user asks otherwise:\n"
        "1. vsh_search to discover relevant commands\n"
        "2. vsh_get_schema before calling unfamiliar commands\n"
        "3. vsh_snapshot_workspace before simulate/approve/execute\n"
        "4. vsh_simulate for reads and dry-runs; never skip simulation for writes\n"
        "5. vsh_approve then vsh_execute_approved only when policy allows\n"
        "Prefer safe read-only commands first. Summarize tool results briefly."
    )


def _tool_names_from_history(result: object) -> list[str]:
    from pydantic_ai.agent import AgentRunResult

    assert isinstance(result, AgentRunResult)
    names: list[str] = []
    for message in result.all_messages():
        for part in getattr(message, "parts", []):
            if isinstance(part, ToolCallPart):
                names.append(part.tool_name)
    return names


def main() -> None:
    parser = argparse.ArgumentParser(description="Run a pydantic-ai agent with vsh tools.")
    parser.add_argument(
        "--mode",
        choices=("scripted", "tools", "live"),
        default="scripted",
        help="scripted: deterministic tool flow; tools: list registered tools; live: real model",
    )
    parser.add_argument(
        "--model",
        default=None,
        help="Live model id (overrides MODEL_NAME env var)",
    )
    args = parser.parse_args()

    with tempfile.TemporaryDirectory(prefix="vsh-pydantic-ai-") as tmp:
        workspace = Path(tmp)
        _prepare_workspace(workspace)
        instructions = (
            _live_instructions()
            if args.mode == "live"
            else (
                "You operate on a workspace using vsh structured commands. "
                "Discover commands, snapshot the workspace, then simulate safe reads."
            )
        )

        if args.mode == "live":
            model: str | TestModel | FunctionModel = _resolve_live_model(args.model)
        elif args.mode == "tools":
            model = TestModel(call_tools=[])
        else:
            model = _build_scripted_model()

        agent, vsh = create_vsh_agent(model, workspace, instructions=instructions)
        deps = vsh.deps

        print("workspace:", deps.workspace_root)
        print("mode:", args.mode)
        if args.mode == "live":
            print("model:", model)

        if args.mode == "tools":
            agent.run_sync("What vsh tools are registered?", deps=deps)
            assert isinstance(model, TestModel)
            params = model.last_model_request_parameters
            print(
                "registered tools:",
                [tool.name for tool in params.function_tools] if params else [],
            )
            return

        result = agent.run_sync("Run the vsh workspace validation flow.", deps=deps)

        print("tools called:", _tool_names_from_history(result))
        print("output:", result.output)
        print("snapshot_id:", deps.snapshot_id)
        print("last_plan_id:", deps.last_plan_id)


if __name__ == "__main__":
    main()
