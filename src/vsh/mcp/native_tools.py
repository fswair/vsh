"""One compact MCP tool over the PyO3-backed VSH runtime."""

from __future__ import annotations

import base64
import os
import time
from dataclasses import dataclass
from functools import lru_cache, wraps
from pathlib import Path
from typing import Literal, TypedDict

from vsh import (
    BashConfig,
    BashResult,
    ExecutionBudget,
    Language,
    Receipt,
    ReceiptDetail,
    RunMode,
    RunRequest,
    Runtime,
)
from vsh._async import native_call
from vsh._native import _Cancellation

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
    max_evidence_records: int
    max_evidence_bytes: int
    max_output_bytes: int
    max_result_bytes: int
    max_exception_bytes: int


@lru_cache(maxsize=16)
def _runtime_for(
    workspace: str,
    policy: PolicyName,
    worker_identity: str | None,
    bash_enabled: bool = False,
    bash_worker: str | None = None,
) -> Runtime:
    # ``worker_identity`` intentionally participates in cache identity. Runtime.open resolves
    # the trusted path itself, including wheel-local scripts, and the model cannot override it.
    del worker_identity
    return Runtime.open(
        workspace, policy=policy, bash=BashConfig(worker_path=bash_worker) if bash_enabled else None
    )


def _bounded_text(value: str) -> tuple[str, bool]:
    if len(value) <= _MAX_INLINE_CHARS:
        return value, False
    return f"{value[:_MAX_INLINE_CHARS]}…", True


def _receipt_payload(receipt: Receipt) -> dict[str, object]:
    result_repr, result_truncated = _bounded_text(receipt.result_repr)
    stdout, stdout_truncated = _bounded_text(receipt.stdout)
    stderr, stderr_truncated = _bounded_text(receipt.stderr)
    bash_result = receipt.result if isinstance(receipt.result, BashResult) else None
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
        "stderr": stderr,
        "stderr_truncated": stderr_truncated,
        "language": "bash" if receipt.language is Language.BASH else "monty",
        "bash": None
        if bash_result is None
        else {
            "profile": bash_result.profile,
            "exit_code": bash_result.exit_code,
            "encoding": "base64",
            "stdout": base64.b64encode(bash_result.stdout[:_MAX_INLINE_CHARS]).decode("ascii"),
            "stderr": base64.b64encode(bash_result.stderr[:_MAX_INLINE_CHARS]).decode("ascii"),
            "output_truncated": len(bash_result.stdout) > _MAX_INLINE_CHARS
            or len(bash_result.stderr) > _MAX_INLINE_CHARS,
        },
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


@dataclass(frozen=True)
class _PreparedRun:
    runtime: Runtime
    request: RunRequest | str

    def execute(self, cancellation: _Cancellation) -> Receipt:
        if isinstance(self.request, str):
            return self.runtime._commit(self.request, time.time_ns() // 1_000_000, cancellation)
        return self.runtime._run(self.request, cancellation)


def _prepare_run(
    code: str | None = None,
    *,
    language: Literal["monty", "bash"] = "monty",
    transaction: str | None = None,
    workspace_root: str | None = None,
    intent: str | None = None,
    mode: RunModeName = "preview",
    policy: PolicyName = "balanced",
    detail: DetailName = "compact",
    budget: BudgetOverrides | None = None,
) -> _PreparedRun:
    if mode not in {"preview", "auto"}:
        raise ValueError(f"unknown run mode: {mode!r}")
    if detail not in {"compact", "full"}:
        raise ValueError(f"unknown receipt detail: {detail!r}")
    if policy not in {"balanced", "strict", "paranoid"}:
        raise ValueError(f"unknown policy profile: {policy!r}")
    if language not in {"monty", "bash"}:
        raise ValueError(f"unknown language: {language!r}")
    bash_enabled = os.environ.get("VSH_ENABLE_BASH") == "1"
    if language == "bash" and not bash_enabled:
        raise ValueError("Bash is not host-enabled; the server host must set VSH_ENABLE_BASH=1")

    workspace = Path(workspace_root or os.getcwd()).resolve(strict=True)
    if not workspace.is_dir():
        raise NotADirectoryError(f"workspace root is not a directory: {workspace}")

    runtime = _runtime_for(
        str(workspace),
        policy,
        os.environ.get("VSH_MONTY_WORKER"),
        bash_enabled,
        os.environ.get("VSH_BASH_WORKER"),
    )
    if transaction is not None:
        if code is not None:
            raise ValueError("pass either code or a preview transaction, not both")
        if mode != "auto":
            raise ValueError("a preview transaction can only be resumed with mode='auto'")
        return _PreparedRun(runtime, transaction)
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
            max_evidence_records=budget.get("max_evidence_records"),
            max_evidence_bytes=budget.get("max_evidence_bytes"),
            max_output_bytes=budget.get("max_output_bytes"),
            max_result_bytes=budget.get("max_result_bytes"),
            max_exception_bytes=budget.get("max_exception_bytes"),
        )
    request = RunRequest(
        code,
        language=Language.BASH if language == "bash" else Language.MONTY,
        intent=intent,
        mode=RunMode.AUTO if mode == "auto" else RunMode.PREVIEW,
        detail=ReceiptDetail.FULL if detail == "full" else ReceiptDetail.COMPACT,
        budget=native_budget,
    )
    return _PreparedRun(runtime, request)


def vsh_run(
    code: str | None = None,
    *,
    language: Literal["monty", "bash"] = "monty",
    transaction: str | None = None,
    workspace_root: str | None = None,
    intent: str | None = None,
    mode: RunModeName = "preview",
    policy: PolicyName = "balanced",
    detail: DetailName = "compact",
    budget: BudgetOverrides | None = None,
) -> dict[str, object]:
    """Execute host-enabled Monty or Bash code against one Rust VirtualFs transaction.

    ``preview`` never changes host files. A later call may promote its exact artifact by passing
    the returned ``transaction`` with ``mode="auto"`` and no code. Otherwise ``auto`` executes and
    commits only a deterministic native auto-approval in one call. Denied or escalated
    transactions remain non-mutating. The receipt is compact and JSON-safe, while all simulation,
    policy, revalidation, and commit semantics stay inside the Rust core.
    """
    prepared = _prepare_run(
        code,
        language=language,
        transaction=transaction,
        workspace_root=workspace_root,
        intent=intent,
        mode=mode,
        policy=policy,
        detail=detail,
        budget=budget,
    )
    return _receipt_payload(prepared.execute(_Cancellation()))


@wraps(vsh_run)
async def _vsh_run_async(
    code: str | None = None,
    *,
    language: Literal["monty", "bash"] = "monty",
    transaction: str | None = None,
    workspace_root: str | None = None,
    intent: str | None = None,
    mode: RunModeName = "preview",
    policy: PolicyName = "balanced",
    detail: DetailName = "compact",
    budget: BudgetOverrides | None = None,
    discard_read_only_preview: bool = False,
) -> dict[str, object]:
    # Opening the runtime can spawn a worker and touch durable state too.
    prepared = await native_call(
        lambda: _prepare_run(
            code,
            language=language,
            transaction=transaction,
            workspace_root=workspace_root,
            intent=intent,
            mode=mode,
            policy=policy,
            detail=detail,
            budget=budget,
        )
    )
    token = _Cancellation()
    receipt = await native_call(
        lambda: prepared.execute(token),
        token,
        on_cancel=lambda receipt: prepared.runtime._cancel_receipt(receipt.transaction),
    )
    payload = _receipt_payload(receipt)
    retained = receipt.state in {"auto_approved", "pending_approval"}
    if (
        discard_read_only_preview
        and receipt.state == "auto_approved"
        and receipt.changed_paths == 0
    ):
        retained = not prepared.runtime.discard_preview(receipt.transaction)
    payload["preview_retained"] = retained
    return payload


__all__ = ("BudgetOverrides", "vsh_run")
