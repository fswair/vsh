from __future__ import annotations as _annotations

__all__ = (
    "DriftError",
    "PolicyRejection",
    "ProtectedPathError",
    "ValidationFailure",
    "VshError",
    "WorkspaceEscape",
)


class VshError(Exception):
    """Base error for structured vsh failures."""

    error_code: str = "vsh_error"

    def __init__(self, message: str, *, hint: str | None = None) -> None:
        super().__init__(message)
        self.hint = hint


class ValidationFailure(VshError):  # noqa: N818
    error_code = "validation_error"


class PolicyRejection(VshError):  # noqa: N818
    error_code = "policy_reject"


class WorkspaceEscape(VshError):  # noqa: N818
    error_code = "workspace_escape"


class ProtectedPathError(VshError):
    error_code = "protected_path"


class DriftError(VshError):
    error_code = "drift_stale"
