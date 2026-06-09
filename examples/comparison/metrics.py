from __future__ import annotations as _annotations

import json
from collections.abc import Iterable
from dataclasses import dataclass

from genai_prices import calc_price
from pydantic_ai.messages import ModelResponse, ToolReturnPart
from pydantic_ai.usage import RunUsage

__all__ = (
    "AgentRunMetrics",
    "compare_metrics",
    "estimate_history_bytes",
    "estimate_tool_return_bytes",
    "model_ref_for_pricing",
    "request_usage_breakdown",
    "usage_cost_usd",
)


@dataclass(frozen=True, slots=True)
class AgentRunMetrics:
    mode: str
    duration_ms: float
    usage: RunUsage
    tool_names: list[str]
    tool_calls: list[dict[str, object]]
    request_usages: list[dict[str, int]]
    history_bytes: int
    tool_return_bytes: int
    tool_return_count: int
    output: str
    validation_passed: bool
    validation_errors: tuple[str, ...]
    cost_usd: float | None

    @property
    def input_tokens(self) -> int:
        return self.usage.input_tokens

    @property
    def output_tokens(self) -> int:
        return self.usage.output_tokens

    @property
    def total_tokens(self) -> int:
        return self.usage.total_tokens

    @property
    def tool_call_count(self) -> int:
        return self.usage.tool_calls


@dataclass(frozen=True, slots=True)
class MetricsComparison:
    vsh: AgentRunMetrics
    native: AgentRunMetrics
    input_token_savings_pct: float
    output_token_savings_pct: float
    total_token_savings_pct: float
    history_byte_savings_pct: float
    duration_savings_pct: float
    cost_savings_pct: float | None
    vsh_faster: bool
    vsh_cheaper: bool | None
    both_passed: bool


def estimate_history_bytes(messages: object) -> int:
    try:
        encoded = json.dumps(messages, default=str).encode("utf-8")
    except TypeError:
        return len(repr(messages).encode("utf-8"))
    return len(encoded)


def estimate_tool_return_bytes(messages: Iterable[object]) -> tuple[int, int]:
    total = 0
    count = 0
    for message in messages:
        for part in getattr(message, "parts", []):
            if not isinstance(part, ToolReturnPart):
                continue
            count += 1
            total += len(json.dumps(part.content, default=str).encode("utf-8"))
    return total, count


def request_usage_breakdown(messages: Iterable[object]) -> list[dict[str, int]]:
    rows: list[dict[str, int]] = []
    for message in messages:
        if not isinstance(message, ModelResponse):
            continue
        usage = message.usage
        rows.append(
            {
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "total_tokens": usage.total_tokens,
            }
        )
    return rows


def model_ref_for_pricing(model_name: str) -> str:
    if ":" in model_name:
        return model_name.split(":", 1)[1]
    return model_name


def usage_cost_usd(usage: RunUsage, model_name: str) -> float | None:
    try:
        calculation = calc_price(usage, model_ref_for_pricing(model_name))
    except Exception:  # noqa: BLE001 — pricing tables may not list every model
        return None
    return float(calculation.total_price)


def compare_metrics(vsh: AgentRunMetrics, native: AgentRunMetrics) -> MetricsComparison:
    def savings_pct(vsh_value: float, native_value: float) -> float:
        if native_value <= 0:
            return 0.0
        return ((native_value - vsh_value) / native_value) * 100.0

    cost_savings: float | None = None
    vsh_cheaper: bool | None = None
    if vsh.cost_usd is not None and native.cost_usd is not None and native.cost_usd > 0:
        cost_savings = savings_pct(vsh.cost_usd, native.cost_usd)
        vsh_cheaper = vsh.cost_usd < native.cost_usd

    return MetricsComparison(
        vsh=vsh,
        native=native,
        input_token_savings_pct=savings_pct(float(vsh.input_tokens), float(native.input_tokens)),
        output_token_savings_pct=savings_pct(float(vsh.output_tokens), float(native.output_tokens)),
        total_token_savings_pct=savings_pct(float(vsh.total_tokens), float(native.total_tokens)),
        history_byte_savings_pct=savings_pct(float(vsh.history_bytes), float(native.history_bytes)),
        duration_savings_pct=savings_pct(vsh.duration_ms, native.duration_ms),
        cost_savings_pct=cost_savings,
        vsh_faster=vsh.duration_ms < native.duration_ms,
        vsh_cheaper=vsh_cheaper,
        both_passed=vsh.validation_passed and native.validation_passed,
    )
