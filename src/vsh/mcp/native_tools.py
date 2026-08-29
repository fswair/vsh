"""One compact MCP tool over the PyO3-backed VSH runtime."""

from __future__ import annotations

import os
import time
from functools import lru_cache
from pathlib import Path
from typing import Literal, TypedDict

from vsh import ExecutionBudget, Receipt, ReceiptDetail, RunMode, RunRequest, Runtime

RunModeName = Literal["preview", "auto"]
PolicyName = Literal["balanced", "strict", "paranoid"]
DetailName = Literal["compact", "full"]

_MAX_INLINE_CHARS = 64 * 1024


class BudgetOverrides(TypedDict, total=False):
    """Optional native execution-budget overrides accepted by ``vsh_run``."""

    max_program_bytes: int
    max_duration_ms: int
    max_recursion_depth: int
    max_memory_bytes: int
    max_os_calls: int
    max_read_bytes: int
    max_write_bytes: int
    max_io_call_bytes: int
    max_path_bytes: int
    max_directory_entries: int
    max_output_bytes: int
    max_result_bytes: int
    max_exception_bytes: int


@lru_cache(maxsize=16)
def _runtime_for(workspace: str, policy: PolicyName, worker_identity: str | None) -> Runtime:
    # ``worker_identity`` intentionally participates in cache identity. Runtime.open resolves
    # the trusted path itself, including wheel-local scripts, and the model cannot override it.
    del worker_identity
    return Runtime.open(workspace, policy=policy)


def _bounded_text(value: str) -> tuple[str, bool]:
    if len(value) <= _MAX_INLINE_CHARS:
        return value, False
    return f"{value[:_MAX_INLINE_CHARS]}…", True


def _receipt_payload(receipt: Receipt) -> dict[str, object]:
    result_repr, result_truncated = _bounded_text(receipt.result_repr)
    stdout, stdout_truncated = _bounded_text(receipt.stdout)
    return {
        "transaction": receipt.transaction,
        "base_snapshot": receipt.base_snapshot,
        "state": receipt.state,
        "decision": receipt.decision,
        "diff": receipt.diff,
        "changed_paths": receipt.changed_paths,
        "changes": [{"path": path, "kind": kind} for path, kind in receipt.changes],
        "result_repr": result_repr,
        "result_truncated": result_truncated,
        "stdout": stdout,
        "stdout_truncated": stdout_truncated,
        "risk_flags": list(receipt.risk_flags),
        "deny_reason": receipt.deny_reason,
        "execution": {
            "os_calls": receipt.os_calls,
            "read_bytes": receipt.read_bytes,
            "write_bytes": receipt.write_bytes,
            "directory_entries": receipt.directory_entries,
            "output_bytes": receipt.output_bytes,
            "denied_accesses": receipt.denied_accesses,
            "result_bytes": receipt.result_bytes,
        },
        "commit": {
            "committed": receipt.committed,
            "operations": receipt.commit_operations,
            "verified_paths": receipt.verified_paths,
            "cleanup_pending": receipt.cleanup_pending,
        },
        "timings_ns": dict(receipt.timings_ns()),
    }


def vsh_run(
    code: str | None = None,
    *,
    transaction: str | None = None,
    workspace_root: str | None = None,
    intent: str | None = None,
    mode: RunModeName = "preview",
    policy: PolicyName = "balanced",
    detail: DetailName = "compact",
    budget: BudgetOverrides | None = None,
) -> dict[str, object]:
    """Execute Monty code against one Rust VirtualFs transaction.

    ``preview`` never changes host files. A later call may promote its exact artifact by passing
    the returned ``transaction`` with ``mode="auto"`` and no code. Otherwise ``auto`` executes and
    commits only a deterministic native auto-approval in one call. Denied or escalated
    transactions remain non-mutating. The receipt is compact and JSON-safe, while all simulation,
    policy, revalidation, and commit semantics stay inside the Rust core.
    """
    if mode not in {"preview", "auto"}:
        raise ValueError(f"unknown run mode: {mode!r}")
    if detail not in {"compact", "full"}:
        raise ValueError(f"unknown receipt detail: {detail!r}")
    if policy not in {"balanced", "strict", "paranoid"}:
        raise ValueError(f"unknown policy profile: {policy!r}")

    workspace = Path(workspace_root or os.getcwd()).resolve(strict=True)
    if not workspace.is_dir():
        raise NotADirectoryError(f"workspace root is not a directory: {workspace}")

    runtime = _runtime_for(str(workspace), policy, os.environ.get("VSH_MONTY_WORKER"))
    if transaction is not None:
        if code is not None:
            raise ValueError("pass either code or a preview transaction, not both")
        if mode != "auto":
            raise ValueError("a preview transaction can only be resumed with mode='auto'")
        now_unix_ms = time.time_ns() // 1_000_000
        return _receipt_payload(runtime.commit(transaction, now_unix_ms))
    if code is None:
        raise ValueError("code is required unless a preview transaction is supplied")

    if budget is None:
        native_budget = ExecutionBudget()
    else:
        native_budget = ExecutionBudget(
            max_program_bytes=budget.get("max_program_bytes"),
            max_duration_ms=budget.get("max_duration_ms"),
            max_recursion_depth=budget.get("max_recursion_depth"),
            max_memory_bytes=budget.get("max_memory_bytes"),
            max_os_calls=budget.get("max_os_calls"),
            max_read_bytes=budget.get("max_read_bytes"),
            max_write_bytes=budget.get("max_write_bytes"),
            max_io_call_bytes=budget.get("max_io_call_bytes"),
            max_path_bytes=budget.get("max_path_bytes"),
            max_directory_entries=budget.get("max_directory_entries"),
            max_output_bytes=budget.get("max_output_bytes"),
            max_result_bytes=budget.get("max_result_bytes"),
            max_exception_bytes=budget.get("max_exception_bytes"),
        )
    request = RunRequest(
        code,
        intent=intent,
        mode=RunMode.AUTO if mode == "auto" else RunMode.PREVIEW,
        detail=ReceiptDetail.FULL if detail == "full" else ReceiptDetail.COMPACT,
        budget=native_budget,
    )
    return _receipt_payload(runtime.run(request))


__all__ = ("BudgetOverrides", "vsh_run")
