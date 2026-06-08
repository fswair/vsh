from __future__ import annotations as _annotations

from pydantic import BaseModel, ConfigDict, Field, SerializeAsAny

from vsh.effects import ActualEffects, RevalidationReport
from vsh.schemas import StructuredCommand
from vsh.simulate.approval_levels import ApprovalTier
from vsh.simulate.models import AccessJournal, PolicyDecision, PredictedEffects


class SimulationResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    plan_id: str
    command: SerializeAsAny[StructuredCommand]
    shell_preview: str
    decision: PolicyDecision
    reason: str | None = None
    raw_matches_shell_preview: bool | None = None
    execution_eligible: bool
    execution_eligibility_reason: str | None = None
    approval_tier: ApprovalTier = "read_only"
    requires_manual_approval: bool = False
    predicted_effects: PredictedEffects
    journal: AccessJournal
    simulation_time_ms: float | None = Field(
        default=None,
        description="Wall-clock time spent simulating the command.",
    )


class ApprovalToken(BaseModel):
    model_config = ConfigDict(extra="forbid")

    token: str
    plan_id: str
    approved_at_ns: int


class PlanRecord(BaseModel):
    model_config = ConfigDict(extra="forbid")

    plan_id: str
    snapshot_id: str
    result: SimulationResult
    created_at_ns: int
    plan_fingerprint: str = ""
    path_fingerprints: dict[str, str] = Field(default_factory=dict)
    approval_token: ApprovalToken | None = None
    executed_at_ns: int | None = None


class ExecutionResult(BaseModel):
    model_config = ConfigDict(extra="forbid")

    plan_id: str
    approval_token: str
    shell_preview: str
    decision: PolicyDecision
    execution_eligible: bool
    applied: bool
    reason: str | None = None
    revalidation: RevalidationReport = Field(default_factory=RevalidationReport)
    actual_effects: ActualEffects | None = None
    matches_prediction: bool | None = None
    total_time_ms: float | None = Field(
        default=None,
        description="Wall-clock time for the full execute_approved pipeline.",
    )
    revalidation_time_ms: float | None = Field(
        default=None,
        description="Time spent revalidating plan drift before execution.",
    )
    apply_time_ms: float | None = Field(
        default=None,
        description="Time spent applying the command to the real filesystem.",
    )
