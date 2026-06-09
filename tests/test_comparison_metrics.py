from __future__ import annotations as _annotations

import sys
from pathlib import Path

from pydantic_ai.usage import RunUsage

EXAMPLES = Path(__file__).resolve().parent.parent / "examples"
if str(EXAMPLES) not in sys.path:
    sys.path.insert(0, str(EXAMPLES))

from comparison.metrics import (  # noqa: E402
    AgentRunMetrics,
    MultiRunSummary,
    compare_metrics,
    summarize_multi_run,
)
from comparison.validation import validate_vsh_history  # noqa: E402


def _metrics(mode: str, input_tokens: int, tool_calls: int) -> AgentRunMetrics:
    usage = RunUsage(input_tokens=input_tokens, output_tokens=1, requests=1, tool_calls=tool_calls)
    return AgentRunMetrics(
        mode=mode,
        duration_ms=10.0,
        usage=usage,
        tool_names=["apply_batch"],
        tool_calls=[],
        request_usages=[],
        history_bytes=100,
        tool_return_bytes=10,
        tool_return_count=1,
        output="ok",
        validation_passed=True,
        validation_errors=(),
        cost_usd=None,
    )


def test_validate_vsh_history_requires_single_apply_batch() -> None:
    good_calls = [
        {
            "tool": "apply_batch",
            "args": {
                "steps": [
                    {"tool_name": "vsh_mkdir"},
                    {"tool_name": "vsh_echo"},
                    {"tool_name": "vsh_grep"},
                    {"tool_name": "vsh_echo"},
                    {"tool_name": "vsh_list"},
                ]
            },
        }
    ]
    assert validate_vsh_history(["apply_batch"], good_calls) == []
    assert validate_vsh_history(["apply_batch", "apply_batch"]) == [
        "expected exactly 1 apply_batch, got 2"
    ]


def test_validate_vsh_history_rejects_shell_like_batch_steps() -> None:
    bad_calls = [
        {
            "tool": "apply_batch",
            "args": {
                "steps": [
                    {"tool_name": "vsh_mkdir"},
                    {"tool_name": "vsh_run_command"},
                ]
            },
        }
    ]
    assert validate_vsh_history(["apply_batch"], bad_calls) == [
        "apply_batch used shell-like steps instead of structured vsh tools",
        "unexpected apply_batch step sequence: ['vsh_mkdir', 'vsh_run_command']",
    ]


def test_summarize_multi_run_median() -> None:
    vsh_one = _metrics("vsh_codemode", 100, 1)
    native = _metrics("native_fs_tools", 200, 2)
    vsh_two = _metrics("vsh_codemode", 300, 3)
    comparisons = [
        compare_metrics(vsh_one, native),
        compare_metrics(vsh_two, native),
    ]
    summary = summarize_multi_run(comparisons)
    assert isinstance(summary, MultiRunSummary)
    assert summary.runs == 2
    assert summary.vsh_input_tokens_median == 200.0
    assert summary.vsh_tool_calls_median == 2.0
