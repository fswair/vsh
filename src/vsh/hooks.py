"""Evidence-first Python coordination for native VSH commit hooks."""

from __future__ import annotations

import inspect
import time
from collections.abc import Awaitable, Callable
from os import PathLike
from typing import TypeAlias, overload

from ._native import (
    CommitPreparation,
    CommitResolution,
    ExecutionBudget,
    HookDecision,
    HookScope,
    Receipt,
    ReceiptDetail,
    RecoveryReport,
    RequestEvent,
    RunMode,
    RunRequest,
    Runtime,
)

HookResult: TypeAlias = HookDecision | Awaitable[HookDecision]
HookHandler: TypeAlias = Callable[[RequestEvent], HookResult]

_ACTIONABLE_STATES = frozenset(("auto_approved", "pending_approval"))


class HookedRuntime:
    """A native runtime coordinated with a sync or async Python hook handler."""

    def __init__(self, runtime: Runtime, hook_handler: HookHandler) -> None:
        self._runtime = runtime
        self._hook_handler = hook_handler

    @classmethod
    def open(
        cls,
        workspace: str | PathLike[str],
        *,
        hook_handler: HookHandler,
        hook_scope: HookScope = HookScope.REVIEW_REQUIRED,
        hook_id: str = "vsh.python-hook",
        review_content_bytes: int = 0,
        data_directory: str | PathLike[str] | None = None,
        policy: str = "balanced",
        worker_path: str | PathLike[str] | None = None,
    ) -> HookedRuntime:
        """Open a runtime whose direct native commit path enforces the hook."""

        runtime = Runtime.open(
            workspace,
            data_directory=data_directory,
            policy=policy,
            worker_path=worker_path,
            hook_id=hook_id,
            hook_scope=hook_scope,
            review_content_bytes=review_content_bytes,
        )
        return cls(runtime, hook_handler)

    @property
    def native(self) -> Runtime:
        """Return the guarded native runtime for advanced prepare/resolve workflows."""

        return self._runtime

    @overload
    def preview(self, request: RunRequest) -> Receipt: ...

    @overload
    def preview(
        self,
        request: str,
        *,
        intent: str | None = None,
        detail: ReceiptDetail | None = None,
        budget: ExecutionBudget | None = None,
    ) -> Receipt: ...

    def preview(
        self,
        request: RunRequest | str,
        *,
        intent: str | None = None,
        detail: ReceiptDetail | None = None,
        budget: ExecutionBudget | None = None,
    ) -> Receipt:
        """Simulate without invoking the hook or changing workspace files."""

        if isinstance(request, RunRequest):
            return self._runtime.preview(request)
        return self._runtime.preview(request, intent=intent, detail=detail, budget=budget)

    def run(self, request: RunRequest, *, now_unix_ms: int | None = None) -> Receipt:
        """Run synchronously; AUTO invokes a synchronous handler when in scope."""

        receipt = self._runtime.preview(request)
        if request.mode is not RunMode.AUTO or receipt.state not in _ACTIONABLE_STATES:
            return receipt
        return self.commit(receipt.transaction, now_unix_ms=now_unix_ms).receipt

    async def arun(self, request: RunRequest, *, now_unix_ms: int | None = None) -> Receipt:
        """Run with support for either synchronous or awaitable hook results."""

        receipt = self._runtime.preview(request)
        if request.mode is not RunMode.AUTO or receipt.state not in _ACTIONABLE_STATES:
            return receipt
        resolution = await self.acommit(receipt.transaction, now_unix_ms=now_unix_ms)
        return resolution.receipt

    def commit(
        self,
        transaction: str,
        *,
        now_unix_ms: int | None = None,
    ) -> CommitResolution:
        """Invoke a synchronous handler and resolve the exact prepared event."""

        preparation = self._runtime.prepare_commit(transaction)
        event = preparation.event
        if event is None:
            decision = HookDecision.follow_policy()
        else:
            try:
                result = self._hook_handler(event)
                if inspect.isawaitable(result):
                    close = getattr(result, "close", None)
                    if close is not None:
                        close()
                    raise TypeError("hook handler returned an awaitable; use acommit() or arun()")
                decision = self._require_decision(result)
            except BaseException:
                self._fail_closed(preparation)
                raise
        return self._runtime.resolve_commit(
            preparation,
            decision,
            self._now(now_unix_ms),
        )

    async def acommit(
        self,
        transaction: str,
        *,
        now_unix_ms: int | None = None,
    ) -> CommitResolution:
        """Await a handler when needed, then resolve the exact prepared event."""

        preparation = self._runtime.prepare_commit(transaction)
        event = preparation.event
        if event is None:
            decision = HookDecision.follow_policy()
        else:
            try:
                result = self._hook_handler(event)
                if inspect.isawaitable(result):
                    result = await result
                decision = self._require_decision(result)
            except BaseException:
                self._fail_closed(preparation)
                raise
        return self._runtime.resolve_commit(
            preparation,
            decision,
            self._now(now_unix_ms),
        )

    def approve(
        self,
        transaction: str,
        principal: str,
        issued_at_unix_ms: int,
        expires_at_unix_ms: int,
    ) -> str:
        """Bind an independent principal to a pending transaction."""

        return self._runtime.approve(
            transaction,
            principal,
            issued_at_unix_ms,
            expires_at_unix_ms,
        )

    def discard_preview(self, transaction: str) -> bool:
        """Discard one process-local preview."""

        return self._runtime.discard_preview(transaction)

    def recover(self) -> RecoveryReport:
        """Recover interrupted durable commits."""

        return self._runtime.recover()

    def transaction_state(self, transaction: str) -> str:
        """Return the durable state of one exact transaction."""

        return self._runtime.transaction_state(transaction)

    @staticmethod
    def _require_decision(result: object) -> HookDecision:
        if not isinstance(result, HookDecision):
            raise TypeError("hook handler must return HookDecision")
        return result

    def _fail_closed(self, preparation: CommitPreparation) -> None:
        self._runtime.fail_hook(preparation)

    @staticmethod
    def _now(value: int | None) -> int:
        return time.time_ns() // 1_000_000 if value is None else value


__all__ = ("HookHandler", "HookResult", "HookedRuntime")
