#!/usr/bin/env python3
"""Live pydantic-ai agent demo: artifact spill + execution_reason.

Requires a real model (OpenRouter, OpenAI, etc.) via MODEL_NAME in .env.

Setup:
    cp .env.example .env
    # set MODEL_NAME and OPENROUTER_API_KEY (or your provider key)

Run:
    uv run python examples/artifact_spill_demo.py
    uv run python examples/artifact_spill_demo.py --model openrouter:anthropic/claude-sonnet-4
    uv run python examples/artifact_spill_demo.py --workspace ./playground/demo-ws --spill-bytes 1024
    uv run python examples/artifact_spill_demo.py --policy-only   # no agent, pure Python policy demo
"""

# ruff: noqa: E402

from __future__ import annotations as _annotations

import argparse
import os
import shutil
import sys
import tempfile
from pathlib import Path
from typing import Any, cast

import logfire
from dotenv import load_dotenv

load_dotenv()

logfire.configure()
logfire.instrument_pydantic_ai()

from pydantic_ai.agent import AgentRunResult
from pydantic_ai.messages import ModelRequest, ToolCallPart, ToolReturnPart

from vsh.agent import VshCapability, create_vsh_agent
from vsh.artifacts import MemoryArtifactStore
from vsh.schemas import TouchCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def _prepare_workspace(root: Path, *, file_count: int) -> None:
    src = root / "src"
    src.mkdir(parents=True, exist_ok=True)
    (root / "README.md").write_text(
        "# Artifact spill demo workspace\n\nGenerated for vsh agent demo.\n",
        encoding="utf-8",
    )
    for index in range(file_count):
        (src / f"module_{index:03d}.py").write_text(
            f"# module {index}\nNEEDLE = 'artifact-demo-{index}'\n",
            encoding="utf-8",
        )


def _resolve_model(cli_model: str | None) -> str:
    model = cli_model or os.environ.get("MODEL_NAME")
    if model is None:
        print("Set MODEL_NAME in .env or pass --model.", file=sys.stderr)
        raise SystemExit(1)
    return model


def _artifact_instructions() -> str:
    return """\
You operate on a workspace using vsh structured commands only.

Artifact spill:
- Large vsh tool outputs may return an ArtifactRef (artifact_id, preview, byte_size) instead of full JSON.
- Use vsh_get_artifact(artifact_id, offset=0, limit=...) to read slices of spilled content.
- Use vsh_index_artifact to tag important artifacts and vsh_search_artifacts to find them later.

execution_reason:
- Every mutation or destructive vsh_simulate call MUST include execution_reason
  (in params or the vsh_simulate execution_reason argument).
- Simulation rejects mutations without it.

Workflow:
1. vsh_snapshot_workspace
2. vsh_search / vsh_get_schema when needed
3. vsh_simulate for reads first; if you get ArtifactRef, fetch details with vsh_get_artifact
4. vsh_simulate mutations only with execution_reason; do not approve/execute unless asked

Be concise in the final answer. Report artifact_id values you spilled or indexed.
"""


def _default_user_prompt() -> str:
    return """\
Work through this checklist on the workspace:

1. Snapshot the workspace.
2. Run a recursive grep for "artifact-demo" under src/ via vsh_simulate.
   If the simulate result is an ArtifactRef (not a full simulation dict), call
   vsh_get_artifact for the first 500 bytes and summarize the preview.
3. Index that artifact with title "grep artifact-demo hits" and tags ["demo", "grep"].
4. Search artifacts for "grep" and confirm your index entry appears.
5. Simulate creating src/agent-notes.txt with vsh_touch TWICE:
   - first WITHOUT execution_reason (expect rejection — report the reason),
   - then WITH execution_reason explaining why the file is needed.

Do not approve or execute any mutation. End with a short summary of spill + policy behavior.
"""


def _tool_names_from_history(result: AgentRunResult[object]) -> list[str]:
    names: list[str] = []
    for message in result.all_messages():
        for part in getattr(message, "parts", []):
            if isinstance(part, ToolCallPart):
                names.append(part.tool_name)
    return names


def _artifact_refs_from_history(result: AgentRunResult[object]) -> list[dict[str, Any]]:
    refs: list[dict[str, Any]] = []
    for message in result.all_messages():
        if not isinstance(message, ModelRequest):
            continue
        for part in message.parts:
            if not isinstance(part, ToolReturnPart):
                continue
            content = part.content
            if isinstance(content, dict) and "artifact_id" in content and "content_hash" in content:
                refs.append(cast(dict[str, Any], content))
    return refs


def _simulate_returns_from_history(result: AgentRunResult[object]) -> list[dict[str, Any]]:
    payloads: list[dict[str, Any]] = []
    for message in result.all_messages():
        if not isinstance(message, ModelRequest):
            continue
        for part in message.parts:
            if not isinstance(part, ToolReturnPart) or part.tool_name != "vsh_simulate":
                continue
            content = part.content
            if isinstance(content, dict) and "plan_id" in content:
                payloads.append(cast(dict[str, Any], content))
    return payloads


def demo_policy_only(workspace: Path) -> None:
    print("\n=== execution_reason policy (Python only, no agent) ===\n")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    reject = simulate_command(TouchCommand(path="scratch.txt"), snapshot)
    print("without execution_reason:")
    print("  decision:", reject.decision)
    print("  reason:  ", reject.reason)
    ok = simulate_command(
        TouchCommand(path="scratch.txt", execution_reason="Bootstrap scratch file for demo"),
        snapshot,
    )
    print("\nwith execution_reason:")
    print("  decision:", ok.decision)
    print("  tier:    ", ok.approval_tier)


def run_live_agent(
    workspace: Path,
    *,
    model: str,
    spill_bytes: int,
    user_prompt: str,
    cleanup_workspace: bool,
) -> None:
    store = MemoryArtifactStore()
    capability = VshCapability(
        workspace,
        artifact_store=store,
        artifact_spill_bytes=spill_bytes,
    )

    agent, vsh = create_vsh_agent(
        model,
        workspace,
        vsh=capability,
        instructions=_artifact_instructions(),
    )
    deps = vsh.deps

    print("\n=== Live agent: artifact spill + execution_reason ===\n")
    print("workspace:   ", deps.workspace_root)
    print("model:       ", model)
    print("spill_bytes: ", spill_bytes)
    print("store:       ", type(store).__name__)
    print()

    result = agent.run_sync(user_prompt, deps=deps)

    tools = _tool_names_from_history(result)
    refs = _artifact_refs_from_history(result)
    sims = _simulate_returns_from_history(result)

    print("tools called:", tools)
    print("artifact refs in history:", len(refs))
    for ref in refs:
        print(
            "  -",
            ref.get("artifact_id"),
            f"({ref.get('byte_size')} bytes, tool={ref.get('tool_name')})",
        )

    print("\nsimulate results:")
    for sim in sims:
        command = sim.get("command", {})
        reason = command.get("execution_reason") if isinstance(command, dict) else None
        print(
            "  -",
            sim.get("decision"),
            command.get("path") if isinstance(command, dict) else command,
            f"execution_reason={reason!r}" if reason else "",
        )
        if sim.get("reason"):
            print("    policy reason:", sim.get("reason"))

    print("\nagent output:\n", result.output)
    print("\nstore records:", len(store._records))  # noqa: SLF001
    print("snapshot_id:", deps.snapshot_id)
    print("last_plan_id:", deps.last_plan_id)

    if cleanup_workspace:
        shutil.rmtree(workspace, ignore_errors=True)


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Live pydantic-ai demo for artifact spill and execution_reason.",
    )
    parser.add_argument(
        "--model",
        default=None,
        help="Model id (default: MODEL_NAME from .env)",
    )
    parser.add_argument(
        "--workspace",
        type=Path,
        default=None,
        help="Use an existing workspace directory (default: temp dir with generated files)",
    )
    parser.add_argument(
        "--file-count",
        type=int,
        default=80,
        help="Generated src/*.py files when using a temp workspace (default: 80)",
    )
    parser.add_argument(
        "--spill-bytes",
        type=int,
        default=1024,
        help="artifact_spill_bytes on VshAgentDeps (default: 1024)",
    )
    parser.add_argument(
        "--prompt",
        default=None,
        help="Override the default user prompt",
    )
    parser.add_argument(
        "--policy-only",
        action="store_true",
        help="Run execution_reason policy demo only (no LLM / no API key)",
    )
    args = parser.parse_args()

    cleanup = args.workspace is None
    if args.workspace is not None:
        workspace = args.workspace.resolve()
        workspace.mkdir(parents=True, exist_ok=True)
    else:
        tmp = tempfile.mkdtemp(prefix="vsh-artifact-live-")
        workspace = Path(tmp)
        _prepare_workspace(workspace, file_count=args.file_count)

    if args.policy_only:
        demo_policy_only(workspace)
        if cleanup:
            shutil.rmtree(workspace, ignore_errors=True)
        return

    model = _resolve_model(args.model)
    run_live_agent(
        workspace,
        model=model,
        spill_bytes=args.spill_bytes,
        user_prompt=args.prompt or _default_user_prompt(),
        cleanup_workspace=cleanup,
    )

    print("\nDone. See docs/ARTIFACTS.md for reference.")


if __name__ == "__main__":
    main()
