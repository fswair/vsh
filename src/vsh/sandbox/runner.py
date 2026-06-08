from __future__ import annotations as _annotations

from typing import Any

from pydantic_monty import (
    CollectString,
    Monty,
    MontyRuntimeError,
    MontySyntaxError,
    MountDir,
    ResourceLimits,
)

from vsh.perf.timing import elapsed_ms, perf_counter_ns
from vsh.registry import get_schema as registry_get_schema
from vsh.registry import registrations
from vsh.registry import search as registry_search
from vsh.runtime import runtime
from vsh.simulate.engine import simulate_command
from vsh.snapshot.models import WorkspaceSnapshot

from .models import SandboxCallRecord, SandboxResult
from .policy import (
    SandboxPolicy,
    classify_simulation_effects,
    mount_mode_for_policy,
    policy_allows_simulation,
)
from .snapshot_advance import advance_snapshot

__all__ = (
    "SandboxPolicyError",
    "run_vsh_sandbox",
)

_MONTY_STUBS = """
def search(query: str) -> list: ...
def get_schema(name: str) -> dict: ...
def simulate(tool_name: str, params: dict) -> dict: ...
"""


class SandboxPolicyError(ValueError):
    """Raised when sandbox code attempts an operation blocked by SandboxPolicy."""


def _monty_runtime_error_message(exc: MontyRuntimeError) -> str:
    message = str(exc)
    prefix = "ValueError: "
    if message.startswith(prefix):
        return message[len(prefix) :]
    return message


class VshSandboxSession:
    __slots__ = (
        "_calls",
        "_policy",
        "_print_collector",
        "_snapshot",
    )

    def __init__(
        self,
        snapshot: WorkspaceSnapshot,
        *,
        policy: SandboxPolicy,
        print_collector: CollectString,
    ) -> None:
        self._snapshot = snapshot
        self._policy: SandboxPolicy = policy
        self._print_collector = print_collector
        self._calls: list[SandboxCallRecord] = []

    @property
    def calls(self) -> list[SandboxCallRecord]:
        return list(self._calls)

    def search(self, query: str) -> list[dict[str, Any]]:
        return [spec.model_dump() for spec in registry_search(query)]

    def get_schema(self, name: str) -> dict[str, Any]:
        return registry_get_schema(name)

    def simulate(self, tool_name: str, params: dict[str, Any]) -> dict[str, Any]:
        registration = registrations[tool_name]
        command = registration.schema_model(**params)
        result = simulate_command(command, self._snapshot)
        effect_kinds = classify_simulation_effects(
            reads=result.predicted_effects.reads,
            creates=result.predicted_effects.creates,
            updates=result.predicted_effects.updates,
            deletes=result.predicted_effects.deletes,
            renames=result.predicted_effects.renames,
            content_reads=result.journal.content_reads or None,
        )
        if not policy_allows_simulation(self._policy, effect_kinds):
            msg = (
                f"sandbox policy {self._policy!r} blocks effects {sorted(effect_kinds)!r} "
                f"for {tool_name}"
            )
            raise SandboxPolicyError(msg)
        if result.decision == "reject":
            msg = result.reason or f"simulation rejected for {tool_name}"
            raise SandboxPolicyError(msg)

        record = SandboxCallRecord(
            tool_name=tool_name,
            params=params,
            plan_id=result.plan_id,
            shell_preview=result.shell_preview,
            decision=result.decision,
            reason=result.reason,
            execution_eligible=result.execution_eligible,
            simulation_time_ms=result.simulation_time_ms,
        )
        self._calls.append(record)
        self._snapshot = advance_snapshot(self._snapshot, result)
        return result.model_dump()

    def external_functions(self) -> dict[str, Any]:
        return {
            "search": self.search,
            "get_schema": self.get_schema,
            "simulate": self.simulate,
        }

    def mount(self) -> MountDir:
        root = self._snapshot.session.workspace_root
        return MountDir(
            "/workspace",
            root,
            mode=mount_mode_for_policy(self._policy),
        )


def run_vsh_sandbox(
    code: str,
    snapshot_id: str,
    *,
    policy: SandboxPolicy = "read_only",
    max_duration_secs: float = 5.0,
) -> SandboxResult:
    """Execute Monty sandbox code with vsh discovery/simulate helpers."""
    start_ns = perf_counter_ns()
    snapshot = runtime.get_snapshot(snapshot_id)
    print_collector = CollectString()
    session = VshSandboxSession(snapshot, policy=policy, print_collector=print_collector)

    monty: Monty
    try:
        monty = Monty(code, type_check=False, type_check_stubs=_MONTY_STUBS)
    except MontySyntaxError as exc:
        return SandboxResult(
            output=None,
            stdout="",
            policy=policy,
            calls=[],
            execution_time_ms=elapsed_ms(start_ns),
            snapshot_id=snapshot_id,
            error=str(exc),
        )

    limits = ResourceLimits(max_duration_secs=max_duration_secs)
    try:
        output = monty.run(
            external_functions=session.external_functions(),
            mount=session.mount(),
            print_callback=print_collector,
            limits=limits,
        )
    except MontyRuntimeError as exc:
        return SandboxResult(
            output=None,
            stdout=print_collector.output,
            policy=policy,
            calls=session.calls,
            execution_time_ms=elapsed_ms(start_ns),
            snapshot_id=snapshot_id,
            error=_monty_runtime_error_message(exc),
        )
    except Exception as exc:
        return SandboxResult(
            output=None,
            stdout=print_collector.output,
            policy=policy,
            calls=session.calls,
            execution_time_ms=elapsed_ms(start_ns),
            snapshot_id=snapshot_id,
            error=str(exc),
        )

    return SandboxResult(
        output=output,
        stdout=print_collector.output,
        policy=policy,
        calls=session.calls,
        execution_time_ms=elapsed_ms(start_ns),
        snapshot_id=snapshot_id,
        error=None,
    )
