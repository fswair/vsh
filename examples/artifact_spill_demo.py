#!/usr/bin/env python3
"""Artifact spill + execution_reason demo for vsh pydantic-ai agents.

Run:
    uv run python examples/artifact_spill_demo.py
    uv run python examples/artifact_spill_demo.py --section reason
    uv run python examples/artifact_spill_demo.py --section store
    uv run python examples/artifact_spill_demo.py --section spill --spill-bytes 256
"""

# ruff: noqa: E402

from __future__ import annotations as _annotations

import argparse
import json
import tempfile
from pathlib import Path
from typing import Any

from pydantic_ai.messages import ModelRequest, ModelResponse, TextPart, ToolCallPart, ToolReturnPart
from pydantic_ai.models.function import FunctionModel

from vsh.agent import VshAgentDeps, VshCapability, _artifact_spill, create_vsh_agent
from vsh.artifacts import MemoryArtifactStore
from vsh.schemas import TouchCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def _banner(title: str) -> None:
    line = "=" * len(title)
    print(f"\n{title}\n{line}")


def _prepare_large_grep_workspace(root: Path, *, file_count: int = 80) -> None:
    src = root / "src"
    src.mkdir()
    (root / "README.md").write_text("# Artifact demo\n", encoding="utf-8")
    for index in range(file_count):
        (src / f"module_{index:03d}.py").write_text(
            f"# module {index}\nVALUE = 'needle-{index}'\n",
            encoding="utf-8",
        )


def _find_spilled_artifact_id(messages: object) -> str | None:
    if not isinstance(messages, list):
        return None
    for message in reversed(messages):
        if not isinstance(message, ModelRequest):
            continue
        for part in message.parts:
            if not isinstance(part, ToolReturnPart):
                continue
            if part.tool_name != "vsh_simulate":
                continue
            content = part.content
            if isinstance(content, dict) and "artifact_id" in content:
                artifact_id = content["artifact_id"]
                if isinstance(artifact_id, str):
                    return artifact_id
    return None


def _build_spill_script() -> FunctionModel:
    """Scripted model: snapshot → large grep → get/index/search artifacts."""
    step = 0

    def next_step(messages: object, _info: object) -> ModelResponse:
        nonlocal step
        step += 1
        if step == 1:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_snapshot_workspace",
                        args={},
                        tool_call_id="demo-snapshot",
                    )
                ]
            )
        if step == 2:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_simulate",
                        args={
                            "tool_name": "vsh_grep",
                            "params": {
                                "pattern": "needle",
                                "path": "src",
                                "recursive": True,
                            },
                        },
                        tool_call_id="demo-grep",
                    )
                ]
            )
        artifact_id = _find_spilled_artifact_id(messages) or "0000000000000000"
        if step == 3:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_get_artifact",
                        args={"artifact_id": artifact_id, "offset": 0, "limit": 400},
                        tool_call_id="demo-get",
                    )
                ]
            )
        if step == 4:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_index_artifact",
                        args={
                            "artifact_id": artifact_id,
                            "title": "grep needle hits",
                            "tags": ["demo", "grep", "src"],
                        },
                        tool_call_id="demo-index",
                    )
                ]
            )
        if step == 5:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_search_artifacts",
                        args={"query": "grep"},
                        tool_call_id="demo-search",
                    )
                ]
            )
        return ModelResponse(parts=[TextPart(content="artifact spill demo complete")])

    return FunctionModel(next_step)


def _extract_last_tool_return(result: object, tool_name: str) -> Any:
    from pydantic_ai.agent import AgentRunResult

    assert isinstance(result, AgentRunResult)
    for message in reversed(result.all_messages()):
        if not isinstance(message, ModelRequest):
            continue
        for part in message.parts:
            if isinstance(part, ToolReturnPart) and part.tool_name == tool_name:
                return part.content
    msg = f"no tool return found for {tool_name}"
    raise RuntimeError(msg)


def demo_store_api() -> None:
    _banner("Direct ArtifactStore API")
    store = MemoryArtifactStore()
    payload = json.dumps({"rows": [{"id": index} for index in range(200)]}).encode()
    record = store.put(
        tool_name="vsh_simulate",
        payload=payload,
        content_type="application/json",
        plan_id="plan_demo",
    )
    print("put:", record.ref.artifact_id, "bytes:", record.ref.byte_size)
    print("preview:", record.ref.preview[:80], "...")
    indexed = store.index(record.ref.artifact_id, title="sim output", tags=["demo"])
    print("index title:", indexed.title, "tags:", indexed.tags)
    hits = store.search("demo")
    print("search hits:", [entry.artifact_id for entry in hits])
    slice_bytes = store.read_bytes(record.ref.artifact_id, offset=0, limit=32)
    print("read_bytes[0:32]:", slice_bytes)


def demo_execution_reason(workspace: Path) -> None:
    _banner("execution_reason policy")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    without = simulate_command(TouchCommand(path="scratch.txt"), snapshot)
    print("without execution_reason:")
    print("  decision:", without.decision)
    print("  reason:  ", without.reason)

    with_reason = simulate_command(
        TouchCommand(path="scratch.txt", execution_reason="Create agent scratch file"),
        snapshot,
    )
    print("\nwith execution_reason:")
    print("  decision:", with_reason.decision)
    print("  tier:    ", with_reason.approval_tier)
    print("  command: ", with_reason.command.execution_reason)


def demo_spill_flow(workspace: Path, *, spill_bytes: int) -> None:
    _banner("Agent artifact spill flow")
    store = MemoryArtifactStore()
    capability = VshCapability(workspace)
    capability.deps.artifact_store = store
    capability.deps.artifact_spill_bytes = spill_bytes
    deps = capability.deps

    agent, _ = create_vsh_agent(_build_spill_script(), workspace, vsh=capability)

    print("workspace:   ", deps.workspace_root)
    print("spill_bytes: ", spill_bytes)

    result = agent.run_sync("Run full artifact spill demo.", deps=deps)

    grep_return = _extract_last_tool_return(result, "vsh_simulate")
    print("\n[vsh_simulate → vsh_grep]")
    if isinstance(grep_return, dict) and "artifact_id" in grep_return:
        print("  spilled! artifact_id:", grep_return["artifact_id"])
        print("  byte_size:", grep_return["byte_size"])
        print("  preview:", str(grep_return.get("preview", ""))[:120], "...")
    else:
        print("  NOT spilled — try --spill-bytes 256 or --file-count 120")
        if isinstance(grep_return, dict):
            print("  decision:", grep_return.get("decision"))

    get_return = _extract_last_tool_return(result, "vsh_get_artifact")
    if isinstance(get_return, dict):
        print("\n[vsh_get_artifact]")
        print("  truncated:", get_return.get("truncated"))
        content = str(get_return.get("content", ""))
        print("  content[0:120]:", content[:120], "...")

    search_return = _extract_last_tool_return(result, "vsh_search_artifacts")
    if isinstance(search_return, list):
        print("\n[vsh_search_artifacts]")
        print("  hits:", len(search_return))
        if search_return:
            print("  first title:", search_return[0].get("title"))

    print("\nfinal output:", result.output)
    print("store records:", len(store._records))  # noqa: SLF001
    print("is_spillable(vsh_simulate):", _artifact_spill.is_spillable_vsh_tool("vsh_simulate"))


def demo_mutation_via_agent(workspace: Path) -> None:
    _banner("Agent vsh_simulate + execution_reason kwarg")
    step = 0

    def next_step(_messages: object, _info: object) -> ModelResponse:
        nonlocal step
        step += 1
        if step == 1:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_snapshot_workspace",
                        args={},
                        tool_call_id="snap",
                    )
                ]
            )
        if step == 2:
            return ModelResponse(
                parts=[
                    ToolCallPart(
                        tool_name="vsh_simulate",
                        args={
                            "tool_name": "vsh_touch",
                            "params": {"path": "agent-notes.txt"},
                            "execution_reason": "Create notes file for agent session",
                        },
                        tool_call_id="touch-ok",
                    )
                ]
            )
        return ModelResponse(parts=[TextPart(content="mutation demo ok")])

    deps = VshAgentDeps.from_path(workspace)
    agent, _ = create_vsh_agent(FunctionModel(next_step), workspace)
    result = agent.run_sync("Simulate touch with reason.", deps=deps)
    sim_return = _extract_last_tool_return(result, "vsh_simulate")
    if isinstance(sim_return, dict):
        print("decision:", sim_return.get("decision"))
        command = sim_return.get("command", {})
        if isinstance(command, dict):
            print("execution_reason:", command.get("execution_reason"))
    print("output:", result.output)


def main() -> None:
    parser = argparse.ArgumentParser(description="Artifact spill + execution_reason demo.")
    parser.add_argument(
        "--section",
        choices=("all", "store", "reason", "spill", "mutation"),
        default="all",
        help="Which demo section to run (default: all)",
    )
    parser.add_argument(
        "--spill-bytes",
        type=int,
        default=512,
        help="artifact_spill_bytes override for the spill demo (default: 512)",
    )
    parser.add_argument(
        "--file-count",
        type=int,
        default=80,
        help="Number of source files to generate for grep spill (default: 80)",
    )
    args = parser.parse_args()

    with tempfile.TemporaryDirectory(prefix="vsh-artifact-demo-") as tmp:
        workspace = Path(tmp)
        _prepare_large_grep_workspace(workspace, file_count=args.file_count)

        if args.section in ("all", "store"):
            demo_store_api()
        if args.section in ("all", "reason"):
            demo_execution_reason(workspace)
        if args.section in ("all", "spill"):
            demo_spill_flow(workspace, spill_bytes=args.spill_bytes)
        if args.section in ("all", "mutation"):
            demo_mutation_via_agent(workspace)

    print("\nDone. See docs/ARTIFACTS.md for the full reference.")


if __name__ == "__main__":
    main()
