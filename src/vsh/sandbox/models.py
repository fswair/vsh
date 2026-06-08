from __future__ import annotations as _annotations

from typing import Any

from pydantic import BaseModel, ConfigDict, Field

from .policy import SandboxPolicy

__all__ = (
    "SandboxCallRecord",
    "SandboxResult",
)


class SandboxCallRecord(BaseModel):
    model_config = ConfigDict(extra="forbid")

    tool_name: str
    params: dict[str, Any]
    plan_id: str
    shell_preview: str
    decision: str
    reason: str | None = None
    execution_eligible: bool
    simulation_time_ms: float | None = None


class SandboxResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    output: Any = None
    stdout: str = ""
    policy: SandboxPolicy
    calls: list[SandboxCallRecord] = Field(default_factory=list)
    execution_time_ms: float | None = None
    snapshot_id: str
    error: str | None = None
