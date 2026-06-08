from __future__ import annotations as _annotations

from pathlib import Path

from vsh.extensions import extensions
from vsh.plans.approval import approve_plan
from vsh.schemas import MkdirCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace
from vsh.snapshot.models import WorkspaceSnapshot


class _RecordingAnalyzer:
    def __init__(self) -> None:
        self.calls: list[tuple[WorkspaceSnapshot, list[str]]] = []

    def analyze(self, snapshot: WorkspaceSnapshot, touched_paths: list[str]) -> list[str]:
        self.calls.append((snapshot, touched_paths))
        return []


def test_execute_approved_invokes_registered_semantic_analyzer(tmp_path: Path) -> None:
    from vsh.execute import execute_approved

    analyzer = _RecordingAnalyzer()
    extensions.semantic_analyzers.append(analyzer)
    try:
        workspace = tmp_path / "workspace"
        workspace.mkdir()
        snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
        result = simulate_command(MkdirCommand(path="pkg"), snapshot)
        token = approve_plan(result.plan_id)

        execution = execute_approved(token.token)

        assert execution.applied is True
        assert analyzer.calls
    finally:
        extensions.semantic_analyzers.clear()
