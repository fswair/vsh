from __future__ import annotations as _annotations

import os
from typing import Any, Literal

from pydantic import ValidationError

from vsh.execute import execute_approved as execute_recorded_plan
from vsh.mcp.receipts import ApplyReceipt, BatchReceipt
from vsh.plans import approve_plan, auto_approve_plan
from vsh.plans.models import ExecutionResult, SimulationResult
from vsh.registry import get_schema as registry_get_schema
from vsh.registry import registrations
from vsh.registry import search as registry_search
from vsh.runtime import runtime
from vsh.sandbox import SandboxPolicy, run_vsh_sandbox
from vsh.schemas import CommandSpec
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace as build_snapshot_workspace

ErrorCode = Literal[
    "unknown_tool",
    "validation_error",
    "policy_reject",
    "drift_stale",
    "invalid_step",
]

Verbosity = Literal["compact", "full"]


def search(query: str) -> list[CommandSpec]:
    """Find vsh command specs by command name, alias, tag, or description."""
    return registry_search(query)


def get_schema(name: str) -> dict[str, Any]:
    """Return the JSON schema for a vsh structured command."""
    return registry_get_schema(name)


def snapshot_workspace(workspace_root: str | None = None, cwd: str | None = None) -> dict[str, Any]:
    """Create and persist a workspace snapshot graph."""
    root = workspace_root or os.getcwd()
    snapshot = build_snapshot_workspace(root, cwd=cwd)
    root_node = snapshot.nodes.get(snapshot.session.workspace_root)
    return {
        "snapshot_id": snapshot.snapshot_id,
        "session": snapshot.session.model_dump(),
        "generated_at_ns": snapshot.generated_at_ns,
        "node_count": len(snapshot.nodes),
        "root": root_node.model_dump() if root_node is not None else None,
    }


def simulate(
    tool_name: str,
    snapshot_id: str,
    params: dict[str, Any],
    *,
    verbosity: Verbosity = "full",
) -> dict[str, Any]:
    """Simulate a structured command against a workspace snapshot."""
    registration = registrations[tool_name]
    snapshot = runtime.get_snapshot(snapshot_id)
    command = registration.schema_model(**params)
    result = simulate_command(command, snapshot)
    if verbosity == "compact":
        return _compact_simulation(result, snapshot_id=snapshot.snapshot_id)
    return result.model_dump()


def approve(plan_id: str, *, auto: bool = False) -> dict[str, Any]:
    """Approve a persisted simulation plan."""
    token = auto_approve_plan(plan_id) if auto else approve_plan(plan_id)
    return token.model_dump()


def execute_approved(approval_token: str, *, verbosity: Verbosity = "full") -> dict[str, Any]:
    """Execute an approved plan."""
    result = execute_recorded_plan(approval_token)
    if verbosity == "compact":
        return _compact_execution(result, snapshot_id=runtime.latest_snapshot_id)
    return result.model_dump()


def vsh_sandbox(
    code: str,
    snapshot_id: str,
    *,
    policy: SandboxPolicy = "read_only",
    verbosity: Verbosity = "full",
) -> dict[str, Any]:
    """Run Monty sandbox code that chains vsh simulate calls in one batch."""
    result = run_vsh_sandbox(code, snapshot_id, policy=policy)
    if verbosity == "compact":
        return {
            "status": "error" if result.error else "ok",
            "snapshot_id": result.snapshot_id,
            "policy": result.policy,
            "output": result.output,
            "stdout": result.stdout or None,
            "error": result.error,
            "call_count": len(result.calls),
            "calls": [
                {
                    "tool_name": call.tool_name,
                    "plan_id": call.plan_id,
                    "decision": call.decision,
                    "execution_eligible": call.execution_eligible,
                    "reason": call.reason,
                }
                for call in result.calls
            ],
            "execution_time_ms": result.execution_time_ms,
        }
    return result.model_dump()


def apply(
    tool_name: str,
    params: dict[str, Any],
    *,
    workspace_root: str | None = None,
    cwd: str | None = None,
    snapshot_id: str | None = None,
    execution_reason: str | None = None,
    execute: bool = True,
    verbosity: Verbosity = "compact",
) -> dict[str, Any]:
    """Simulate, approve, and optionally execute one vsh command in a single call."""
    snapshot = _ensure_snapshot(workspace_root=workspace_root, cwd=cwd, snapshot_id=snapshot_id)
    try:
        result = _simulate_for_apply(
            tool_name,
            snapshot.snapshot_id,
            params,
            execution_reason=execution_reason,
        )
    except (KeyError, ValueError, ValidationError) as exc:
        return _validated_apply_receipt(
            _compact_error(
                tool_name=tool_name,
                snapshot_id=snapshot.snapshot_id,
                reason=str(exc),
                exc=exc,
            )
        )
    if not execute or not result.execution_eligible:
        return _validated_apply_receipt(
            _compact_apply_result(
                result,
                snapshot_id=snapshot.snapshot_id,
                execution=None,
                verbosity=verbosity,
            )
        )

    token = approve_plan(result.plan_id)
    execution = execute_recorded_plan(token.token)
    return _validated_apply_receipt(
        _compact_apply_result(
            result,
            snapshot_id=runtime.latest_snapshot_id or snapshot.snapshot_id,
            execution=execution,
            verbosity=verbosity,
        )
    )


def apply_batch(
    steps: list[dict[str, Any]],
    *,
    workspace_root: str | None = None,
    cwd: str | None = None,
    snapshot_id: str | None = None,
    continue_on_error: bool = False,
    transactional: bool = False,
    verbosity: Verbosity = "compact",
) -> dict[str, Any]:
    """Run multiple vsh apply steps while reusing the current runtime snapshot.

    Use one ``apply_batch`` call per agent task. Each step needs ``tool_name``,
    ``params`` (use ``path``, ``output_path``, ``text``/``content``, ``pattern``),
    and ``execution_reason`` on mutations. Read steps include ``stdout`` in receipts
    when output is available.
    """
    import tempfile
    from pathlib import Path

    from vsh.execute.rollback import RollbackSession, restore_session

    snapshot = _ensure_snapshot(workspace_root=workspace_root, cwd=cwd, snapshot_id=snapshot_id)
    current_snapshot_id = snapshot.snapshot_id
    receipts: list[dict[str, Any]] = []
    rollback: RollbackSession | None = None
    if transactional:
        rollback = RollbackSession(backup_root=Path(tempfile.mkdtemp(prefix="vsh-rollback-")))

    for index, raw_step in enumerate(steps):
        tool_name = str(raw_step.get("tool_name", ""))
        params = raw_step.get("params", {})
        if not isinstance(params, dict):
            receipt = _compact_error(
                tool_name=tool_name,
                snapshot_id=current_snapshot_id,
                reason="step params must be a dict",
                error_code="invalid_step",
                hint="each step params must be a JSON object",
            )
            receipt["step"] = index
            receipts.append(receipt)
            if not continue_on_error:
                break
            continue
        reason = raw_step.get("execution_reason")
        execution_reason = reason if isinstance(reason, str) else None
        execute_step = raw_step.get("execute", True)
        should_execute = execute_step if isinstance(execute_step, bool) else True

        try:
            result = _simulate_for_apply(
                tool_name,
                current_snapshot_id,
                params,
                execution_reason=execution_reason,
            )
        except (KeyError, ValueError, ValidationError) as exc:
            receipt = _compact_error(
                tool_name=tool_name,
                snapshot_id=current_snapshot_id,
                reason=str(exc),
                exc=exc,
            )
            receipt["step"] = index
            receipts.append(receipt)
            if not continue_on_error:
                if rollback is not None:
                    restore_session(rollback)
                break
            continue
        execution: ExecutionResult | None = None
        if should_execute and result.execution_eligible:
            if rollback is not None:
                for path in _touched_paths(result):
                    rollback.record(Path(path), Path(path).exists())
            token = approve_plan(result.plan_id)
            execution = execute_recorded_plan(token.token)
            current_snapshot_id = runtime.latest_snapshot_id or current_snapshot_id
        receipt = _compact_apply_result(
            result,
            snapshot_id=current_snapshot_id,
            execution=execution,
            verbosity=verbosity,
        )
        if verbosity == "compact":
            receipt.pop("snapshot_id", None)
            receipt.pop("execution_eligible", None)
            receipt.pop("applied", None)
        receipt["step"] = index
        receipts.append(receipt)
        if receipt["status"] not in {"applied", "simulated"} and not continue_on_error:
            if rollback is not None:
                restore_session(rollback)
            break

    return _validated_batch_receipt(
        {
            "status": "ok"
            if all(item["status"] in {"applied", "simulated"} for item in receipts)
            else "error",
            "snapshot_id": current_snapshot_id,
            "completed_steps": len(receipts),
            "steps": receipts,
        }
    )


def _ensure_snapshot(
    *,
    workspace_root: str | None,
    cwd: str | None,
    snapshot_id: str | None,
) -> Any:
    if snapshot_id is not None:
        return runtime.get_snapshot(snapshot_id)
    root = workspace_root or os.getcwd()
    return build_snapshot_workspace(root, cwd=cwd)


def _simulate_for_apply(
    tool_name: str,
    snapshot_id: str,
    params: dict[str, Any],
    *,
    execution_reason: str | None,
) -> SimulationResult:
    tool_name = _normalize_apply_tool_name(tool_name)
    params = _normalize_apply_params(tool_name, params)
    registration = registrations[tool_name]
    merged_params = dict(params)
    if execution_reason is not None and "execution_reason" not in merged_params:
        merged_params["execution_reason"] = execution_reason
    snapshot = runtime.get_snapshot(snapshot_id)
    command = registration.schema_model(**merged_params)
    return simulate_command(command, snapshot)


def _normalize_apply_tool_name(tool_name: str) -> str:
    aliases = {
        "mkdir": "vsh_mkdir",
        "write": "vsh_echo",
        "write_file": "vsh_echo",
        "vsh_write": "vsh_echo",
        "vsh_write_file": "vsh_echo",
        "vsh_write_text_file": "vsh_echo",
        "echo": "vsh_echo",
        "grep": "vsh_grep",
        "search": "vsh_grep",
        "list": "vsh_list",
        "list_dir": "vsh_list",
        "ls": "vsh_list",
    }
    return aliases.get(tool_name, tool_name)


_PATH_ALIASES = ("dir", "directory", "folder", "dir_path")
_GREP_PATH_ALIASES = ("root", "root_dir", "root_directory", "search_path", "parent")
_ECHO_OUTPUT_ALIASES = ("output_file", "dest", "filepath", "file", "file_path", "filename")


def _sanitize_path_value(value: str) -> str:
    return value.strip()


def _pop_path_alias(params: dict[str, Any], aliases: tuple[str, ...]) -> str | None:
    for key in aliases:
        if key in params:
            value = params.pop(key)
            if isinstance(value, str):
                return _sanitize_path_value(value)
    return None


def _normalize_apply_params(tool_name: str, params: dict[str, Any]) -> dict[str, Any]:
    normalized = dict(params)
    if isinstance(normalized.get("path"), str):
        normalized["path"] = _sanitize_path_value(normalized["path"])
    if isinstance(normalized.get("output_path"), str):
        normalized["output_path"] = _sanitize_path_value(normalized["output_path"])
    if tool_name in {"vsh_mkdir", "vsh_list"} and "path" not in normalized:
        path_value = _pop_path_alias(normalized, _PATH_ALIASES)
        if path_value is not None:
            normalized["path"] = path_value
    if tool_name == "vsh_mkdir" and "recursive" in normalized:
        normalized["parents"] = normalized.pop("recursive")
    if tool_name == "vsh_mkdir" and "parents" not in normalized:
        normalized["parents"] = True
    if tool_name == "vsh_grep" and "path" not in normalized:
        path_value = _pop_path_alias(normalized, _GREP_PATH_ALIASES)
        if path_value is not None:
            normalized["path"] = path_value
    if tool_name == "vsh_echo":
        if "text" not in normalized and "content" in normalized:
            normalized["text"] = normalized.pop("content")
        if "output_path" not in normalized:
            moved = False
            for key in _ECHO_OUTPUT_ALIASES:
                if key in normalized:
                    normalized["output_path"] = normalized.pop(key)
                    moved = True
                    break
            if not moved and "path" in normalized:
                normalized["output_path"] = normalized.pop("path")
        if "output_path" in normalized and "no_newline" not in normalized:
            text = normalized.get("text")
            if isinstance(text, str):
                if text.endswith("\\n"):
                    normalized["text"] = text.removesuffix("\\n")
                else:
                    normalized["text"] = text.removesuffix("\n")
            normalized["no_newline"] = True
    return normalized


def _compact_apply_result(
    result: SimulationResult,
    *,
    snapshot_id: str | None,
    execution: ExecutionResult | None,
    verbosity: Verbosity,
) -> dict[str, Any]:
    if execution is None:
        status = "simulated" if result.execution_eligible else "rejected"
    elif execution.applied:
        status = "applied"
    else:
        status = "execution_failed"

    if verbosity == "full":
        receipt: dict[str, Any] = {
            "status": status,
            "snapshot_id": snapshot_id,
            "plan_id": result.plan_id,
            "tool_name": result.command.__class__.__name__,
            "shell_preview": result.shell_preview,
            "decision": result.decision,
            "approval_tier": result.approval_tier,
            "execution_eligible": result.execution_eligible,
            "reason": result.reason,
            "execution_reason": result.command.execution_reason,
            "touched_paths": _touched_paths(result),
            "simulation_time_ms": result.simulation_time_ms,
        }
        if execution is not None:
            receipt.update(_compact_execution(execution, snapshot_id=snapshot_id))
        receipt["simulation"] = result.model_dump()
        if execution is not None:
            receipt["execution"] = execution.model_dump()
        return receipt

    receipt = {
        "status": status,
        "snapshot_id": snapshot_id,
        "tool_name": result.command.__class__.__name__,
        "execution_eligible": result.execution_eligible,
    }
    if result.reason is not None:
        receipt["reason"] = result.reason
    if execution is not None:
        receipt["applied"] = execution.applied
        if execution.reason is not None:
            receipt["execution_failure_reason"] = execution.reason
        actual = execution.actual_effects
        if actual is not None and actual.stdout is not None:
            receipt["stdout"] = actual.stdout
    return receipt


def _compact_simulation(result: SimulationResult, *, snapshot_id: str) -> dict[str, Any]:
    return {
        "status": "simulated" if result.execution_eligible else "rejected",
        "snapshot_id": snapshot_id,
        "plan_id": result.plan_id,
        "shell_preview": result.shell_preview,
        "decision": result.decision,
        "approval_tier": result.approval_tier,
        "execution_eligible": result.execution_eligible,
        "reason": result.reason,
        "touched_paths": _touched_paths(result),
        "simulation_time_ms": result.simulation_time_ms,
    }


def _compact_execution(
    execution: ExecutionResult,
    *,
    snapshot_id: str | None,
) -> dict[str, Any]:
    actual = execution.actual_effects
    return {
        "snapshot_id": snapshot_id,
        "applied": execution.applied,
        "execution_failure_reason": execution.reason,
        "matches_prediction": execution.matches_prediction,
        "stdout": actual.stdout if actual is not None else None,
        "actual_effect_counts": _actual_effect_counts(actual),
        "total_time_ms": execution.total_time_ms,
        "revalidation_time_ms": execution.revalidation_time_ms,
        "apply_time_ms": execution.apply_time_ms,
    }


def _classify_apply_error(
    exc: BaseException | None,
    *,
    tool_name: str,
    reason: str,
    error_code: ErrorCode | None = None,
) -> tuple[ErrorCode, str | None]:
    if error_code is not None:
        return error_code, None
    if isinstance(exc, KeyError):
        return "unknown_tool", f"use a registered vsh command name instead of {tool_name!r}"
    if isinstance(exc, ValidationError):
        hint = _validation_error_hint(exc, tool_name=tool_name)
        return "validation_error", hint
    lowered = reason.lower()
    if "stale" in lowered or "drift" in lowered:
        return "drift_stale", "refresh snapshot_id and re-simulate before execute"
    if "execution_reason" in lowered or "reject" in lowered or "policy" in lowered:
        return "policy_reject", "include execution_reason for mutation commands"
    return "validation_error", None


def _validation_error_hint(exc: ValidationError, *, tool_name: str) -> str | None:
    for error in exc.errors():
        loc = error.get("loc", ())
        if not loc:
            continue
        field = str(loc[-1])
        if field in _PATH_ALIASES or field in {"directory", "folder", "dir_path"}:
            return f"use params.path instead of params.{field}"
        if field in _GREP_PATH_ALIASES or field in {"root_directory", "search_path", "parent"}:
            return f"use params.path instead of params.{field}"
        if field in _ECHO_OUTPUT_ALIASES or field in {"file_path", "filename"}:
            return f"use params.output_path (and params.text or params.content) for {tool_name}"
    return "check get_schema for required fields"


def _compact_error(
    *,
    tool_name: str,
    snapshot_id: str | None,
    reason: str,
    exc: BaseException | None = None,
    error_code: ErrorCode | None = None,
    hint: str | None = None,
) -> dict[str, Any]:
    code, inferred_hint = _classify_apply_error(
        exc,
        tool_name=tool_name,
        reason=reason,
        error_code=error_code,
    )
    receipt: dict[str, Any] = {
        "status": "error",
        "snapshot_id": snapshot_id,
        "tool_name": tool_name,
        "reason": reason,
        "error_code": code,
        "execution_eligible": False,
        "applied": False,
    }
    resolved_hint = hint if hint is not None else inferred_hint
    if resolved_hint is not None:
        receipt["hint"] = resolved_hint
    return receipt


def _validated_apply_receipt(receipt: dict[str, Any]) -> dict[str, Any]:
    return ApplyReceipt.model_validate(receipt).model_dump(exclude_none=True)


def _validated_batch_receipt(receipt: dict[str, Any]) -> dict[str, Any]:
    return BatchReceipt.model_validate(receipt).model_dump(exclude_none=True)


def _actual_effect_counts(actual: Any) -> dict[str, int] | None:
    if actual is None:
        return None
    return {
        "reads": len(actual.reads),
        "creates": len(actual.creates),
        "updates": len(actual.updates),
        "deletes": len(actual.deletes),
        "renames": len(actual.renames),
    }


def _touched_paths(result: SimulationResult) -> list[str]:
    effects = result.predicted_effects
    paths = (
        list(effects.reads)
        + list(effects.creates)
        + list(effects.updates)
        + list(effects.deletes)
        + [src for src, _dst in effects.renames]
        + [dst for _src, dst in effects.renames]
    )
    return sorted(set(paths))
