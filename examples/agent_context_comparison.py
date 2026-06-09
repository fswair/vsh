#!/usr/bin/env python3
"""Compare vsh CodeMode MCP agent vs native structured FS tools on the same scenario.

Measures duration, token usage (result.usage), approximate history size, and cost.

Run:
    uv run python examples/agent_context_comparison.py
    uv run python examples/agent_context_comparison.py --model openrouter:google/gemini-3-flash-preview
    uv run python examples/agent_context_comparison.py --vsh-toolset  # legacy FunctionToolset
"""

# ruff: noqa: E402

from __future__ import annotations as _annotations

import argparse
import os
import shutil
import sys
import tempfile
from datetime import UTC, datetime
from pathlib import Path

from dotenv import load_dotenv

load_dotenv()

EXAMPLES_DIR = Path(__file__).resolve().parent
if str(EXAMPLES_DIR) not in sys.path:
    sys.path.insert(0, str(EXAMPLES_DIR))

from comparison.metrics import (  # noqa: E402
    AgentRunMetrics,
    MetricsComparison,
    compare_metrics,
    estimate_history_bytes,
    estimate_tool_return_bytes,
    request_usage_breakdown,
    usage_cost_usd,
)
from comparison.native_agent import NativeAgentDeps, create_native_fs_agent  # noqa: E402
from comparison.report import write_comparison_report  # noqa: E402
from comparison.scenario import (  # noqa: E402
    ScenarioPrompts,
    build_scenario_prompts,
    prepare_workspace,
)
from comparison.validation import (  # noqa: E402
    extract_tool_calls,
    extract_tool_names,
    validate_run,
)

from vsh.agent import create_vsh_agent  # noqa: E402
from vsh.perf.timing import elapsed_ms, perf_counter_ns  # noqa: E402


def _resolve_model(cli_model: str | None) -> str:
    model = cli_model or os.environ.get("MODEL_NAME")
    if model is None:
        print("Set MODEL_NAME in .env or pass --model.", file=sys.stderr)
        raise SystemExit(1)
    return model


def _run_vsh_agent(
    workspace: Path,
    model: str,
    *,
    codemode_mcp: bool,
    prompts: ScenarioPrompts,
) -> AgentRunMetrics:
    agent, capability = create_vsh_agent(
        model,
        workspace,
        codemode_mcp=codemode_mcp,
        instructions=prompts.vsh_system_instructions,  # type: ignore[attr-defined]
    )
    deps = capability.deps
    start_ns = perf_counter_ns()
    result = agent.run_sync(prompts.vsh_user_prompt, deps=deps)  # type: ignore[attr-defined]
    duration_ms = elapsed_ms(start_ns)
    messages = result.all_messages()
    tool_names = extract_tool_names(messages)
    tool_calls = extract_tool_calls(messages)
    tool_return_bytes, tool_return_count = estimate_tool_return_bytes(messages)
    validation = validate_run(
        workspace=workspace,
        tool_names=tool_names,
        tool_calls=tool_calls,
        mode="vsh",
    )
    usage = result.usage
    return AgentRunMetrics(
        mode="vsh_codemode" if codemode_mcp else "vsh_toolset",
        duration_ms=duration_ms,
        usage=usage,
        tool_names=tool_names,
        tool_calls=tool_calls,
        request_usages=request_usage_breakdown(messages),
        history_bytes=estimate_history_bytes(messages),
        tool_return_bytes=tool_return_bytes,
        tool_return_count=tool_return_count,
        output=str(result.output),
        validation_passed=validation.passed,
        validation_errors=tuple(validation.errors),
        cost_usd=usage_cost_usd(usage, model),
    )


def _run_native_agent(workspace: Path, model: str, *, prompts: ScenarioPrompts) -> AgentRunMetrics:
    deps = NativeAgentDeps(workspace_root=str(workspace.resolve()))
    agent = create_native_fs_agent(
        model,
        workspace,
        instructions=prompts.native_system_instructions,  # type: ignore[attr-defined]
    )
    start_ns = perf_counter_ns()
    result = agent.run_sync(prompts.native_user_prompt, deps=deps)  # type: ignore[attr-defined]
    duration_ms = elapsed_ms(start_ns)
    messages = result.all_messages()
    tool_names = extract_tool_names(messages)
    tool_calls = extract_tool_calls(messages)
    tool_return_bytes, tool_return_count = estimate_tool_return_bytes(messages)
    validation = validate_run(
        workspace=workspace,
        tool_names=tool_names,
        tool_calls=tool_calls,
        mode="native",
    )
    usage = result.usage
    return AgentRunMetrics(
        mode="native_fs_tools",
        duration_ms=duration_ms,
        usage=usage,
        tool_names=tool_names,
        tool_calls=tool_calls,
        request_usages=request_usage_breakdown(messages),
        history_bytes=estimate_history_bytes(messages),
        tool_return_bytes=tool_return_bytes,
        tool_return_count=tool_return_count,
        output=str(result.output),
        validation_passed=validation.passed,
        validation_errors=tuple(validation.errors),
        cost_usd=usage_cost_usd(usage, model),
    )


def _print_summary(comparison: MetricsComparison) -> None:
    print("\n=== Comparison summary ===\n")
    print(f"both passed: {comparison.both_passed}")
    print(f"vsh duration:    {comparison.vsh.duration_ms:.1f} ms")
    print(f"native duration: {comparison.native.duration_ms:.1f} ms")
    print(f"duration savings: {comparison.duration_savings_pct:.1f}%")
    print(f"vsh input tokens:    {comparison.vsh.input_tokens}")
    print(f"native input tokens: {comparison.native.input_tokens}")
    print(f"input token savings: {comparison.input_token_savings_pct:.1f}%")
    print(f"total token savings: {comparison.total_token_savings_pct:.1f}%")
    print(f"history byte savings: {comparison.history_byte_savings_pct:.1f}%")
    print(f"vsh tool return bytes:    {comparison.vsh.tool_return_bytes}")
    print(f"native tool return bytes: {comparison.native.tool_return_bytes}")


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", default=None, help="Model id (default: MODEL_NAME from .env)")
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=None,
        help="Report directory (default: playground/reports/agent-context-<timestamp>)",
    )
    parser.add_argument(
        "--workspace",
        type=Path,
        default=None,
        help="Reuse an existing workspace directory (default: temp dir)",
    )
    parser.add_argument(
        "--vsh-toolset",
        action="store_true",
        help="Use legacy VshToolset (codemode_mcp=False) instead of CodeMode MCP",
    )
    parser.add_argument(
        "--native-only",
        action="store_true",
        help="Run only the native structured-fs agent",
    )
    parser.add_argument(
        "--vsh-only",
        action="store_true",
        help="Run only the vsh agent",
    )
    args = parser.parse_args(argv)

    model = _resolve_model(args.model)
    prompts = build_scenario_prompts()
    print(f"vsh prompt chars:    {len(prompts.vsh_user_prompt)}")
    print(f"native prompt chars: {len(prompts.native_user_prompt)}")

    cleanup = args.workspace is None
    if args.workspace is not None:
        workspace = args.workspace.resolve()
        workspace.mkdir(parents=True, exist_ok=True)
    else:
        workspace = Path(tempfile.mkdtemp(prefix="vsh-agent-compare-"))
    prepare_workspace(workspace)

    codemode_mcp = not args.vsh_toolset
    metadata = {
        "generated_at": datetime.now(tz=UTC).isoformat(),
        "model": model,
        "codemode_mcp": codemode_mcp,
        "vsh_prompt_chars": len(prompts.vsh_user_prompt),
        "native_prompt_chars": len(prompts.native_user_prompt),
    }

    vsh_metrics: AgentRunMetrics | None = None
    native_metrics: AgentRunMetrics | None = None

    if not args.native_only:
        print("\n--- Running vsh agent ---")
        vsh_metrics = _run_vsh_agent(workspace, model, codemode_mcp=codemode_mcp, prompts=prompts)
        print("vsh validation:", "PASS" if vsh_metrics.validation_passed else "FAIL")
        if vsh_metrics.validation_errors:
            for err in vsh_metrics.validation_errors:
                print(" ", err)

    if not args.vsh_only:
        if vsh_metrics is not None:
            prepare_workspace(workspace)
        print("\n--- Running native structured-fs agent ---")
        native_metrics = _run_native_agent(workspace, model, prompts=prompts)
        print("native validation:", "PASS" if native_metrics.validation_passed else "FAIL")
        if native_metrics.validation_errors:
            for err in native_metrics.validation_errors:
                print(" ", err)

    if vsh_metrics is not None and native_metrics is not None:
        comparison = compare_metrics(vsh_metrics, native_metrics)
        _print_summary(comparison)
        stamp = datetime.now(tz=UTC).strftime("%Y%m%d-%H%M%S")
        output_dir = args.output_dir or (
            Path(__file__).resolve().parent.parent
            / "playground"
            / "reports"
            / f"agent-context-{stamp}"
        )
        md_path, json_path = write_comparison_report(
            output_dir,
            comparison,
            model=model,
            workspace=str(workspace),
            metadata=metadata,
        )
        print(f"\nwritten: {md_path}")
        print(f"written: {json_path}")

    if cleanup:
        shutil.rmtree(workspace, ignore_errors=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
