from __future__ import annotations as _annotations

from typing import Any, Literal

from pydantic import BaseModel, ConfigDict, Field

ReceiptStatus = Literal["applied", "simulated", "rejected", "execution_failed", "error", "ok"]
ErrorCode = Literal[
    "unknown_tool",
    "validation_error",
    "policy_reject",
    "drift_stale",
    "invalid_step",
]


class ApplyReceipt(BaseModel):
    model_config = ConfigDict(extra="allow")

    status: ReceiptStatus
    snapshot_id: str | None = None
    tool_name: str | None = None
    execution_eligible: bool | None = None
    applied: bool | None = None
    reason: str | None = None
    error_code: ErrorCode | None = None
    hint: str | None = None
    stdout: str | None = None


class BatchStepReceipt(ApplyReceipt):
    step: int = Field(ge=0)


class BatchReceipt(BaseModel):
    model_config = ConfigDict(extra="allow")

    status: Literal["ok", "error"]
    snapshot_id: str
    completed_steps: int
    steps: list[dict[str, Any]]
