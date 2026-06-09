from __future__ import annotations as _annotations

from pathlib import Path

import pytest

from vsh.mcp import tools
from vsh.plans import approve_plan
from vsh.plans.models import ExecutionResult
from vsh.runtime import runtime
from vsh.schemas import CatCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_apply_executes_one_command_with_compact_receipt(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    receipt = tools.apply(
        "vsh_mkdir",
        {"path": "bench/output", "parents": True},
        workspace_root=str(workspace),
        cwd=".",
        execution_reason="create benchmark output directory",
    )

    assert receipt["status"] == "applied"
    assert receipt["applied"] is True
    assert receipt["execution_eligible"] is True
    assert "simulation" not in receipt
    assert (workspace / "bench" / "output").is_dir()


def test_apply_rejects_mutation_without_execution_reason(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    receipt = tools.apply(
        "vsh_mkdir",
        {"path": "bench/output", "parents": True},
        workspace_root=str(workspace),
        cwd=".",
    )

    assert receipt["status"] == "rejected"
    assert "applied" not in receipt
    assert receipt["reason"] == "execution_reason is required for mutation commands"
    assert not (workspace / "bench" / "output").exists()


def test_apply_returns_compact_error_for_unknown_tool(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    receipt = tools.apply(
        "vsh_missing",
        {},
        workspace_root=str(workspace),
        cwd=".",
    )

    assert receipt["status"] == "error"
    assert receipt["tool_name"] == "vsh_missing"
    assert receipt["applied"] is False


def test_apply_can_only_simulate_and_return_full_payload(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    receipt = tools.apply(
        "vsh_list",
        {"path": "."},
        workspace_root=str(workspace),
        cwd=".",
        execute=False,
        verbosity="full",
    )

    assert receipt["status"] == "simulated"
    assert receipt["execution_eligible"] is True
    assert "simulation" in receipt
    assert "execution" not in receipt


def test_apply_batch_reuses_updated_snapshot_between_steps(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    result = tools.apply_batch(
        [
            {
                "tool_name": "vsh_mkdir",
                "params": {"path": "bench/output", "parents": True},
                "execution_reason": "create output directory",
            },
            {
                "tool_name": "vsh_echo",
                "params": {
                    "text": "marker: bench-marker-42",
                    "output_path": "bench/output/summary.md",
                    "no_newline": True,
                },
                "execution_reason": "write benchmark summary",
            },
            {
                "tool_name": "vsh_grep",
                "params": {"pattern": "bench-marker-42", "path": ".", "recursive": True},
            },
            {
                "tool_name": "vsh_echo",
                "params": {
                    "text": '{"marker":"bench-marker-42","phase":"complete"}',
                    "output_path": "bench/output/status.json",
                    "no_newline": True,
                },
                "execution_reason": "write benchmark status",
            },
            {
                "tool_name": "vsh_list",
                "params": {"path": "bench/output"},
            },
        ],
        workspace_root=str(workspace),
        cwd=".",
    )

    assert result["status"] == "ok"
    assert result["completed_steps"] == 5
    assert [step["status"] for step in result["steps"]] == ["applied"] * 5
    grep_step = result["steps"][2]
    assert grep_step.get("tool_name") == "GrepCommand"
    grep_stdout = grep_step["stdout"] or ""
    assert "bench-marker-42" in grep_stdout
    assert "summary.md" in grep_stdout
    snapshot = runtime.get_snapshot(result["snapshot_id"])
    assert str((workspace / "bench" / "output" / "summary.md").resolve()) in snapshot.nodes
    assert str((workspace / "bench" / "output" / "status.json").resolve()) in snapshot.nodes


def test_apply_batch_normalizes_common_write_aliases(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    result = tools.apply_batch(
        [
            {
                "tool_name": "vsh_write_file",
                "params": {
                    "content": '{"marker":"bench-marker-42","phase":"complete"}\\n',
                    "dest": "bench/output/status.json",
                },
                "execution_reason": "write benchmark status",
            },
        ],
        workspace_root=str(workspace),
        cwd=".",
    )

    assert result["status"] == "ok"
    assert (workspace / "bench" / "output" / "status.json").read_text(
        encoding="utf-8"
    ) == '{"marker":"bench-marker-42","phase":"complete"}'


def test_apply_batch_normalizes_common_mkdir_and_grep_aliases(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    result = tools.apply_batch(
        [
            {
                "tool_name": "vsh_mkdir",
                "params": {"path": "bench/output"},
                "execution_reason": "create output directory",
            },
            {
                "tool_name": "vsh_echo",
                "params": {
                    "content": "marker: bench-marker-42\\n",
                    "path": "bench/output/summary.md",
                },
                "execution_reason": "write marker",
            },
            {
                "tool_name": "vsh_grep",
                "params": {
                    "pattern": "bench-marker-42",
                    "root": "bench/output",
                    "recursive": True,
                },
            },
        ],
        workspace_root=str(workspace),
        cwd=".",
    )

    assert result["status"] == "ok"
    assert [step["status"] for step in result["steps"]] == ["applied"] * 3


def test_apply_batch_normalizes_dir_and_file_aliases(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    result = tools.apply_batch(
        [
            {
                "tool_name": "vsh_mkdir",
                "params": {"dir": "bench/output", "recursive": True},
                "execution_reason": "create output directory",
            },
            {
                "tool_name": "vsh_echo",
                "params": {
                    "content": "marker: bench-marker-42\\n",
                    "file": "bench/output/summary.md",
                },
                "execution_reason": "write marker",
            },
            {
                "tool_name": "vsh_list",
                "params": {"dir": "bench/output"},
            },
        ],
        workspace_root=str(workspace),
        cwd=".",
    )

    assert result["status"] == "ok"
    assert [step["status"] for step in result["steps"]] == ["applied"] * 3


def test_apply_batch_normalizes_root_dir_alias(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    result = tools.apply_batch(
        [
            {
                "tool_name": "vsh_mkdir",
                "params": {"path": "bench/output"},
                "execution_reason": "create output directory",
            },
            {
                "tool_name": "vsh_echo",
                "params": {
                    "content": "marker: bench-marker-42\\n",
                    "path": "bench/output/summary.md",
                },
                "execution_reason": "write marker",
            },
            {
                "tool_name": "vsh_grep",
                "params": {
                    "pattern": "bench-marker-42",
                    "root_dir": "bench/output",
                    "recursive": True,
                },
            },
        ],
        workspace_root=str(workspace),
        cwd=".",
    )

    assert result["status"] == "ok"
    assert "bench-marker-42" in result["steps"][2]["stdout"]


def test_apply_batch_reports_bad_step_params(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    result = tools.apply_batch(
        [{"tool_name": "vsh_list", "params": "not-a-dict"}],
        workspace_root=str(workspace),
        cwd=".",
    )

    assert result["status"] == "error"
    assert result["completed_steps"] == 1
    assert result["steps"][0]["reason"] == "step params must be a dict"
    assert result["steps"][0]["error_code"] == "invalid_step"


def test_apply_batch_normalizes_baseline_directory_and_file_path_aliases(tmp_path: Path) -> None:
    """Regression for pre-roadmap baseline param aliases that caused agent retries."""
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    result = tools.apply_batch(
        [
            {
                "tool_name": "vsh_mkdir",
                "params": {"directory": "bench/output/", "parents": True},
                "execution_reason": "create output directory",
            },
            {
                "tool_name": "vsh_echo",
                "params": {
                    "content": "marker: bench-marker-42\\n",
                    "file_path": "bench/output/summary.md",
                },
                "execution_reason": "write marker",
            },
            {
                "tool_name": "vsh_grep",
                "params": {
                    "pattern": "bench-marker-42",
                    "root_directory": "bench/output/",
                    "recursive": True,
                },
            },
            {
                "tool_name": "vsh_list",
                "params": {"directory": "bench/output/"},
            },
        ],
        workspace_root=str(workspace),
        cwd=".",
    )

    assert result["status"] == "ok"
    assert [step["status"] for step in result["steps"]] == ["applied"] * 4
    assert "bench-marker-42" in (result["steps"][2]["stdout"] or "")


def test_apply_unknown_tool_includes_error_code_and_hint(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    receipt = tools.apply(
        "vsh_missing",
        {},
        workspace_root=str(workspace),
        cwd=".",
    )

    assert receipt["error_code"] == "unknown_tool"
    assert "hint" in receipt


def test_apply_batch_can_continue_after_error(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    result = tools.apply_batch(
        [
            {"tool_name": "vsh_missing", "params": {}},
            {"tool_name": "vsh_list", "params": {"path": "."}, "execute": False},
        ],
        workspace_root=str(workspace),
        cwd=".",
        continue_on_error=True,
    )

    assert result["status"] == "error"
    assert [step["status"] for step in result["steps"]] == ["error", "simulated"]


def test_apply_batch_can_continue_after_bad_params(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    result = tools.apply_batch(
        [
            {"tool_name": "vsh_list", "params": "not-a-dict"},
            {"tool_name": "vsh_list", "params": {"path": "."}, "execute": False},
        ],
        workspace_root=str(workspace),
        cwd=".",
        continue_on_error=True,
    )

    assert result["status"] == "error"
    assert [step["status"] for step in result["steps"]] == ["error", "simulated"]


def test_apply_batch_stops_after_unknown_tool_when_not_continuing(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    result = tools.apply_batch(
        [
            {"tool_name": "vsh_missing", "params": {}},
            {"tool_name": "vsh_list", "params": {"path": "."}, "execute": False},
        ],
        workspace_root=str(workspace),
        cwd=".",
    )

    assert result["status"] == "error"
    assert result["completed_steps"] == 1


def test_apply_batch_stops_after_rejected_step_when_not_continuing(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    result = tools.apply_batch(
        [
            {"tool_name": "vsh_mkdir", "params": {"path": "bench/output"}},
            {"tool_name": "vsh_list", "params": {"path": "."}, "execute": False},
        ],
        workspace_root=str(workspace),
        cwd=".",
    )

    assert result["status"] == "error"
    assert result["completed_steps"] == 1
    assert result["steps"][0]["status"] == "rejected"


def test_apply_batch_full_verbosity_includes_debug_payloads(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    result = tools.apply_batch(
        [
            {
                "tool_name": "vsh_mkdir",
                "params": {"path": "bench/output"},
                "execution_reason": "create output directory",
            }
        ],
        workspace_root=str(workspace),
        cwd=".",
        verbosity="full",
    )

    step = result["steps"][0]
    assert step["status"] == "applied"
    assert "simulation" in step
    assert "execution" in step
    assert "touched_paths" in step


def test_apply_uses_existing_snapshot_id(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    receipt = tools.apply(
        "vsh_list",
        {"path": "."},
        snapshot_id=snapshot.snapshot_id,
        execute=False,
    )

    assert receipt["status"] == "simulated"
    assert receipt["snapshot_id"] == snapshot.snapshot_id


def test_apply_normalizes_output_file_and_filepath_aliases(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    first = tools.apply(
        "write_file",
        {"content": "one\\n", "output_file": "bench/output/one.txt"},
        workspace_root=str(workspace),
        cwd=".",
        execution_reason="write first file",
    )
    second = tools.apply(
        "vsh_write_text_file",
        {"content": "two\\n", "filepath": "bench/output/two.txt"},
        workspace_root=str(workspace),
        cwd=".",
        execution_reason="write second file",
    )

    assert first["status"] == "applied"
    assert second["status"] == "applied"
    assert (workspace / "bench" / "output" / "one.txt").read_text(encoding="utf-8") == "one"
    assert (workspace / "bench" / "output" / "two.txt").read_text(encoding="utf-8") == "two"


def test_apply_trims_real_newline_for_path_alias(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    receipt = tools.apply(
        "vsh_echo",
        {"text": "one\n", "path": "bench/output/one.txt"},
        workspace_root=str(workspace),
        cwd=".",
        execution_reason="write one file",
    )

    assert receipt["status"] == "applied"
    assert (workspace / "bench" / "output" / "one.txt").read_text(encoding="utf-8") == "one"


def test_apply_echo_without_output_path_only_simulates(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    receipt = tools.apply(
        "vsh_echo",
        {"text": "hello"},
        workspace_root=str(workspace),
        cwd=".",
        execute=False,
    )

    assert receipt["status"] == "simulated"


def test_apply_echo_output_path_without_text_returns_error(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    receipt = tools.apply(
        "vsh_echo",
        {"output_path": "bench/output/missing.txt"},
        workspace_root=str(workspace),
        cwd=".",
        execution_reason="write missing text",
    )

    assert receipt["status"] == "error"


def test_apply_reports_execution_failure(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    def fail_execution(_approval_token: str) -> ExecutionResult:
        return ExecutionResult(
            plan_id="plan_failed",
            approval_token="approval_failed",
            shell_preview="mkdir bench/output",
            decision="approve",
            execution_eligible=True,
            applied=False,
            reason="boom",
        )

    monkeypatch.setattr(tools, "execute_recorded_plan", fail_execution)

    receipt = tools.apply(
        "vsh_mkdir",
        {"path": "bench/output"},
        workspace_root=str(workspace),
        cwd=".",
        execution_reason="create output directory",
    )

    assert receipt["status"] == "execution_failed"
    assert receipt["execution_failure_reason"] == "boom"


def test_vsh_sandbox_supports_compact_verbosity(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    result = tools.vsh_sandbox(
        'simulate("vsh_list", {"path": "."})',
        snapshot.snapshot_id,
        verbosity="compact",
    )

    assert result["status"] == "ok"
    assert result["call_count"] == 1
    assert result["calls"][0]["tool_name"] == "vsh_list"


def test_simulate_and_execute_support_compact_verbosity(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "README.md").write_text("hello\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    sim = tools.simulate(
        "vsh_list",
        snapshot.snapshot_id,
        {"path": "."},
        verbosity="compact",
    )
    token = tools.approve(sim["plan_id"])
    executed = tools.execute_approved(token["token"], verbosity="compact")

    assert sim["status"] == "simulated"
    assert "predicted_effects" not in sim
    assert executed["applied"] is True
    assert executed["snapshot_id"] == snapshot.snapshot_id


def test_execute_compact_reports_stale_plan_without_actual_effects(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "README.md"
    target.write_text("hello\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(CatCommand(path="README.md"), snapshot)
    token = approve_plan(result.plan_id)
    target.write_text("changed\n", encoding="utf-8")

    executed = tools.execute_approved(token.token, verbosity="compact")

    assert executed["applied"] is False
    assert executed["actual_effect_counts"] is None
