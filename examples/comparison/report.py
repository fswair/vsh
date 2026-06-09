from __future__ import annotations as _annotations

import json
from dataclasses import asdict
from datetime import UTC, datetime
from pathlib import Path
from typing import Any

from .metrics import MetricsComparison

__all__ = ("write_comparison_report",)


def _fmt_pct(value: float) -> str:
    sign = "+" if value >= 0 else ""
    return f"{sign}{value:.1f}%"


def _fmt_ms(value: float) -> str:
    return f"{value:.1f} ms"


def _fmt_usd(value: float | None) -> str:
    if value is None:
        return "n/a"
    return f"${value:.6f}"


def render_markdown_report(
    comparison: MetricsComparison,
    *,
    model: str,
    workspace: str,
    metadata: dict[str, Any],
) -> str:
    vsh = comparison.vsh
    native = comparison.native
    lines = [
        "# Agent context comparison: vsh CodeMode vs native structured FS tools",
        "",
        f"- generated: {datetime.now(tz=UTC).isoformat()}",
        f"- model: `{model}`",
        f"- workspace: `{workspace}`",
        "",
        "## Scenario validation",
        "",
        f"- vsh passed: **{vsh.validation_passed}**",
        f"- native passed: **{native.validation_passed}**",
        f"- both passed: **{comparison.both_passed}**",
        "",
    ]
    if vsh.validation_errors:
        lines.extend(["### vsh validation errors", ""])
        lines.extend(f"- {err}" for err in vsh.validation_errors)
        lines.append("")
    if native.validation_errors:
        lines.extend(["### native validation errors", ""])
        lines.extend(f"- {err}" for err in native.validation_errors)
        lines.append("")

    lines.extend(
        [
            "## Duration",
            "",
            "| mode | wall time |",
            "|------|----------:|",
            f"| vsh codemode | {_fmt_ms(vsh.duration_ms)} |",
            f"| native fs tools | {_fmt_ms(native.duration_ms)} |",
            "",
            f"- vsh faster: **{comparison.vsh_faster}**",
            f"- duration savings (vsh vs native): **{_fmt_pct(comparison.duration_savings_pct)}**",
            "",
            "## Token usage (`result.usage`)",
            "",
            "| metric | vsh | native | savings |",
            "|--------|----:|-------:|--------:|",
            f"| input tokens | {vsh.input_tokens} | {native.input_tokens} | {_fmt_pct(comparison.input_token_savings_pct)} |",
            f"| output tokens | {vsh.output_tokens} | {native.output_tokens} | {_fmt_pct(comparison.output_token_savings_pct)} |",
            f"| total tokens | {vsh.total_tokens} | {native.total_tokens} | {_fmt_pct(comparison.total_token_savings_pct)} |",
            f"| model requests | {vsh.usage.requests} | {native.usage.requests} | — |",
            f"| tool calls (usage) | {vsh.tool_call_count} | {native.tool_call_count} | — |",
            "",
            "## Approximate history payload",
            "",
            f"- vsh serialized history: **{vsh.history_bytes}** bytes",
            f"- native serialized history: **{native.history_bytes}** bytes",
            f"- byte savings: **{_fmt_pct(comparison.history_byte_savings_pct)}**",
            f"- vsh tool return payload: **{vsh.tool_return_bytes}** bytes across {vsh.tool_return_count} returns",
            f"- native tool return payload: **{native.tool_return_bytes}** bytes across {native.tool_return_count} returns",
            "",
            "## Per-request usage",
            "",
            f"- vsh request usage: `{json.dumps(vsh.request_usages)}`",
            f"- native request usage: `{json.dumps(native.request_usages)}`",
            "",
            "## Cost estimate (genai-prices)",
            "",
            f"- vsh: {_fmt_usd(vsh.cost_usd)}",
            f"- native: {_fmt_usd(native.cost_usd)}",
        ]
    )
    if comparison.cost_savings_pct is not None:
        lines.append(f"- cost savings: **{_fmt_pct(comparison.cost_savings_pct)}**")
        lines.append(f"- vsh cheaper: **{comparison.vsh_cheaper}**")
    lines.append("")

    lines.extend(
        [
            "## Tool surface",
            "",
            f"- vsh tools called ({len(vsh.tool_names)}): `{', '.join(vsh.tool_names)}`",
            f"- native tools called ({len(native.tool_names)}): `{', '.join(native.tool_names)}`",
            "",
            "### native tool calls",
            "",
        ]
    )
    if native.tool_calls:
        for index, call in enumerate(native.tool_calls, start=1):
            tool = call.get("tool", "?")
            args = call.get("args", {})
            lines.append(f"{index}. `{tool}` args={json.dumps(args, ensure_ascii=False)}")
    else:
        lines.append("_none captured_")
    lines.extend(
        [
            "",
            "## Agent outputs (truncated)",
            "",
            "### vsh",
            "",
            vsh.output[:2000] or "_empty_",
            "",
            "### native",
            "",
            native.output[:2000] or "_empty_",
            "",
            "## Metadata",
            "",
            "```json",
            json.dumps(metadata, indent=2),
            "```",
            "",
        ]
    )
    return "\n".join(lines)


def write_comparison_report(
    output_dir: Path,
    comparison: MetricsComparison,
    *,
    model: str,
    workspace: str,
    metadata: dict[str, Any],
) -> tuple[Path, Path]:
    output_dir.mkdir(parents=True, exist_ok=True)
    markdown_path = output_dir / "comparison.md"
    json_path = output_dir / "comparison.json"
    markdown_path.write_text(
        render_markdown_report(comparison, model=model, workspace=workspace, metadata=metadata),
        encoding="utf-8",
    )
    payload = {
        "model": model,
        "workspace": workspace,
        "metadata": metadata,
        "comparison": {
            "vsh": asdict(comparison.vsh),
            "native": asdict(comparison.native),
            "input_token_savings_pct": comparison.input_token_savings_pct,
            "output_token_savings_pct": comparison.output_token_savings_pct,
            "total_token_savings_pct": comparison.total_token_savings_pct,
            "history_byte_savings_pct": comparison.history_byte_savings_pct,
            "duration_savings_pct": comparison.duration_savings_pct,
            "cost_savings_pct": comparison.cost_savings_pct,
            "vsh_faster": comparison.vsh_faster,
            "vsh_cheaper": comparison.vsh_cheaper,
            "both_passed": comparison.both_passed,
        },
    }
    json_path.write_text(json.dumps(payload, indent=2, default=str), encoding="utf-8")
    return markdown_path, json_path
