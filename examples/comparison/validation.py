from __future__ import annotations as _annotations

import json
from collections.abc import Iterable
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

from pydantic_ai.messages import ToolCallPart

from .native_agent import NATIVE_TOOL_NAMES
from .scenario import MARKER, OUTPUT_DIR, STATUS_FILE, SUMMARY_FILE

__all__ = (
    "RunValidation",
    "extract_tool_calls",
    "extract_tool_names",
    "validate_native_history",
    "validate_vsh_history",
    "validate_workspace",
)


@dataclass(slots=True)
class RunValidation:
    passed: bool
    workspace_errors: list[str] = field(default_factory=list)
    history_errors: list[str] = field(default_factory=list)

    @property
    def errors(self) -> list[str]:
        return [*self.workspace_errors, *self.history_errors]


def validate_workspace(workspace: Path) -> list[str]:
    errors: list[str] = []
    output_dir = workspace / OUTPUT_DIR
    if not output_dir.is_dir():
        errors.append(f"missing directory: {OUTPUT_DIR}")
        return errors

    summary = workspace / SUMMARY_FILE
    if not summary.is_file():
        errors.append(f"missing file: {SUMMARY_FILE}")
    else:
        text = summary.read_text(encoding="utf-8")
        if MARKER not in text:
            errors.append(f"{SUMMARY_FILE} does not contain {MARKER!r}")

    status = workspace / STATUS_FILE
    if not status.is_file():
        errors.append(f"missing file: {STATUS_FILE}")
    else:
        try:
            payload = json.loads(status.read_text(encoding="utf-8"))
        except json.JSONDecodeError:
            errors.append(f"{STATUS_FILE} is not valid JSON")
        else:
            if payload.get("marker") != MARKER:
                errors.append(f"{STATUS_FILE} marker mismatch")
            if payload.get("phase") != "complete":
                errors.append(f"{STATUS_FILE} phase is not complete")

    extra_files = sorted(
        path.relative_to(output_dir)
        for path in output_dir.rglob("*")
        if path.is_file() and path.name not in {SUMMARY_FILE.name, STATUS_FILE.name}
    )
    if extra_files:
        errors.append(f"unexpected files in {OUTPUT_DIR}: {extra_files}")
    return errors


def _tool_call_args(part: ToolCallPart) -> dict[str, Any]:
    args = part.args
    if isinstance(args, dict):
        return args
    if isinstance(args, str):
        try:
            parsed = json.loads(args)
        except json.JSONDecodeError:
            return {}
        return parsed if isinstance(parsed, dict) else {}
    return {}


def _normalize_tool(name: str) -> str:
    if ":" in name:
        return name.rsplit(":", 1)[-1]
    return name


def extract_tool_names(messages: Iterable[object]) -> list[str]:
    names: list[str] = []
    for message in messages:
        for part in getattr(message, "parts", []):
            if isinstance(part, ToolCallPart):
                names.append(_normalize_tool(part.tool_name))
    return names


def extract_tool_calls(messages: Iterable[object]) -> list[dict[str, Any]]:
    calls: list[dict[str, Any]] = []
    for message in messages:
        for part in getattr(message, "parts", []):
            if isinstance(part, ToolCallPart):
                calls.append(
                    {
                        "tool": _normalize_tool(part.tool_name),
                        "args": _tool_call_args(part),
                    }
                )
    return calls


def validate_vsh_history(tool_names: list[str]) -> list[str]:
    errors: list[str] = []
    normalized = [_normalize_tool(name) for name in tool_names]
    if "apply_batch" in normalized:
        return errors
    if "apply" in normalized:
        apply_count = normalized.count("apply")
        if apply_count >= 3:
            return errors
        errors.append(f"expected apply_batch or at least 3 apply calls, got {apply_count}")
        return errors
    if not any(name in {"snapshot_workspace", "vsh_snapshot_workspace"} for name in normalized):
        errors.append("history missing snapshot_workspace")
    sandbox_count = sum(1 for name in normalized if name == "vsh_sandbox")
    simulate_count = sum(1 for name in normalized if name in {"simulate", "vsh_simulate"})
    if simulate_count == 0 and sandbox_count == 0:
        errors.append("history missing simulate or vsh_sandbox")
    if not any(name in {"search", "vsh_search"} for name in normalized):
        errors.append("history missing search/discovery call")
    if simulate_count < 3 and sandbox_count < 2:
        errors.append(
            f"expected >=3 simulate or >=2 vsh_sandbox calls, got simulate={simulate_count} sandbox={sandbox_count}"
        )
    if not any(name in {"approve", "vsh_approve"} for name in normalized):
        errors.append("history missing approve for mutations")
    if not any(name in {"execute_approved", "vsh_execute_approved"} for name in normalized):
        errors.append("history missing execute_approved for mutations")
    return errors


def validate_native_history(tool_names: list[str], tool_calls: list[dict[str, Any]]) -> list[str]:
    errors: list[str] = []
    normalized = [_normalize_tool(name) for name in tool_names]
    used = set(normalized)
    if not used.intersection(NATIVE_TOOL_NAMES):
        errors.append(
            f"history missing native fs tools; expected one of {sorted(NATIVE_TOOL_NAMES)}"
        )
    if "mkdir" not in used:
        errors.append("history missing mkdir")
    write_count = normalized.count("write_file")
    if write_count < 2:
        errors.append(f"expected at least 2 write_file calls, got {write_count}")
    if "grep" not in used:
        errors.append("history missing grep")
    if "list_dir" not in used:
        errors.append("history missing list_dir")

    grep_patterns = [
        str(call["args"].get("pattern", "")) for call in tool_calls if call.get("tool") == "grep"
    ]
    if not any(MARKER in pattern for pattern in grep_patterns):
        errors.append("grep never searched for bench marker pattern")

    write_paths = [
        str(call["args"].get("path", "")) for call in tool_calls if call.get("tool") == "write_file"
    ]
    if not any("summary.md" in path for path in write_paths):
        errors.append("write_file never targeted summary.md")
    if not any("status.json" in path for path in write_paths):
        errors.append("write_file never targeted status.json")
    return errors


def validate_run(
    *,
    workspace: Path,
    tool_names: list[str],
    tool_calls: list[dict[str, Any]] | None,
    mode: str,
) -> RunValidation:
    workspace_errors = validate_workspace(workspace)
    if mode == "vsh":
        history_errors = validate_vsh_history(tool_names)
    else:
        history_errors = validate_native_history(tool_names, tool_calls or [])
    return RunValidation(
        passed=not workspace_errors and not history_errors,
        workspace_errors=workspace_errors,
        history_errors=history_errors,
    )
