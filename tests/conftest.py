from __future__ import annotations as _annotations

import pytest

from vsh.schemas import StructuredCommand

MUTATION_REASON = "test execution rationale"


def with_execution_reason(command: StructuredCommand) -> StructuredCommand:
    if command.execution_reason:
        return command
    return command.model_copy(update={"execution_reason": MUTATION_REASON})


@pytest.fixture(autouse=True)
def disable_disk_persistence(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("VSH_PERSIST", "0")
    monkeypatch.setenv("VSH_ARTIFACT_STORE", "memory")
