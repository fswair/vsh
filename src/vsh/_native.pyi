"""Typed surface of the PyO3-backed VSH native module."""

from __future__ import annotations

from collections.abc import Mapping
from enum import Enum
from os import PathLike
from typing import overload

__version__: str

class VshRuntimeError(RuntimeError):
    """Base exception for typed native VSH failures."""

class VshExecutionError(VshRuntimeError):
    """Monty compilation, execution, or hard-budget failure."""

class VshStateError(VshRuntimeError):
    """Transaction lifecycle, approval, reservation, or replay failure."""

class VshStaleError(VshRuntimeError):
    """Host dependencies changed after virtual execution."""

class VshRecoveryError(VshRuntimeError):
    """Durable recovery is required or could not prove ownership."""

class VshInternalError(VshRuntimeError):
    """A contained internal panic or invariant failure."""

class RunMode(Enum):
    PREVIEW: RunMode
    AUTO: RunMode

class ReceiptDetail(Enum):
    COMPACT: ReceiptDetail
    FULL: ReceiptDetail

class ExecutionBudget:
    def __init__(
        self,
        *,
        max_program_bytes: int | None = ...,
        max_duration_ms: int | None = ...,
        max_recursion_depth: int | None = ...,
        max_memory_bytes: int | None = ...,
        max_os_calls: int | None = ...,
        max_read_bytes: int | None = ...,
        max_write_bytes: int | None = ...,
        max_io_call_bytes: int | None = ...,
        max_path_bytes: int | None = ...,
        max_directory_entries: int | None = ...,
        max_output_bytes: int | None = ...,
        max_result_bytes: int | None = ...,
        max_exception_bytes: int | None = ...,
    ) -> None: ...
    @property
    def max_program_bytes(self) -> int: ...
    @property
    def max_duration_ms(self) -> int: ...
    @property
    def max_recursion_depth(self) -> int: ...
    @property
    def max_memory_bytes(self) -> int: ...
    @property
    def max_os_calls(self) -> int: ...
    @property
    def max_read_bytes(self) -> int: ...
    @property
    def max_write_bytes(self) -> int: ...
    @property
    def max_io_call_bytes(self) -> int: ...
    @property
    def max_path_bytes(self) -> int: ...
    @property
    def max_directory_entries(self) -> int: ...
    @property
    def max_output_bytes(self) -> int: ...
    @property
    def max_result_bytes(self) -> int: ...
    @property
    def max_exception_bytes(self) -> int: ...

class RunRequest:
    def __init__(
        self,
        code: str,
        *,
        intent: str | None = ...,
        mode: RunMode | None = ...,
        detail: ReceiptDetail | None = ...,
        budget: ExecutionBudget | None = ...,
    ) -> None: ...
    @property
    def code(self) -> str: ...
    @property
    def intent(self) -> str | None: ...
    @property
    def mode(self) -> RunMode: ...
    @property
    def detail(self) -> ReceiptDetail: ...
    @property
    def budget(self) -> ExecutionBudget: ...

class Receipt:
    @property
    def transaction(self) -> str: ...
    @property
    def base_snapshot(self) -> str: ...
    @property
    def state(self) -> str: ...
    @property
    def decision(self) -> str: ...
    @property
    def diff(self) -> str: ...
    @property
    def changed_paths(self) -> int: ...
    @property
    def changes(self) -> list[tuple[str, str]]: ...
    @property
    def result(self) -> object: ...
    @property
    def result_repr(self) -> str: ...
    @property
    def stdout(self) -> str: ...
    @property
    def risk_flags(self) -> list[str]: ...
    @property
    def deny_reason(self) -> str | None: ...
    @property
    def os_calls(self) -> int: ...
    @property
    def read_bytes(self) -> int: ...
    @property
    def write_bytes(self) -> int: ...
    @property
    def directory_entries(self) -> int: ...
    @property
    def output_bytes(self) -> int: ...
    @property
    def denied_accesses(self) -> int: ...
    @property
    def result_bytes(self) -> int: ...
    @property
    def committed(self) -> bool: ...
    @property
    def commit_operations(self) -> int | None: ...
    @property
    def verified_paths(self) -> int | None: ...
    @property
    def cleanup_pending(self) -> bool: ...
    def timings_ns(self) -> Mapping[str, int]: ...

class RecoveryReport:
    @property
    def finalized_commits(self) -> int: ...
    @property
    def rolled_back(self) -> int: ...
    @property
    def cleaned(self) -> int: ...
    @property
    def orphaned(self) -> int: ...
    @property
    def conflicts(self) -> list[tuple[str, str | None, str]]: ...

class Runtime:
    @staticmethod
    def open(
        workspace: str | PathLike[str],
        *,
        data_directory: str | PathLike[str] | None = ...,
        policy: str = ...,
        worker_path: str | PathLike[str] | None = ...,
    ) -> Runtime: ...
    def run(self, request: RunRequest) -> Receipt: ...
    @overload
    def preview(self, request: RunRequest) -> Receipt: ...
    @overload
    def preview(
        self,
        request: str,
        *,
        intent: str | None = ...,
        detail: ReceiptDetail | None = ...,
        budget: ExecutionBudget | None = ...,
    ) -> Receipt: ...
    def discard_preview(self, transaction: str) -> bool: ...
    def approve(
        self,
        transaction: str,
        principal: str,
        issued_at_unix_ms: int,
        expires_at_unix_ms: int,
    ) -> str: ...
    def commit(self, transaction: str, now_unix_ms: int) -> Receipt: ...
    def recover(self) -> RecoveryReport: ...

def version() -> str:
    """Return the native VSH semantic version."""

def engine_kind() -> str:
    """Return the native engine identity."""

def normalize_path(path: str) -> str:
    """Normalize a workspace-relative virtual path."""
