from __future__ import annotations as _annotations

import sys
from pathlib import Path

import pytest

EXAMPLES_DIR = Path(__file__).resolve().parent.parent / "examples"
if str(EXAMPLES_DIR) not in sys.path:
    sys.path.insert(0, str(EXAMPLES_DIR))

from comparison.native_agent import (  # pyright: ignore[reportMissingImports]  # noqa: E402
    NATIVE_TOOL_NAMES,
    NativeAgentDeps,
    create_native_fs_agent,
)
from comparison.native_path_guard import (  # pyright: ignore[reportMissingImports]  # noqa: E402
    NativePathError,
    validate_mkdir_path,
    validate_write_path,
)
from pydantic_ai.models.test import TestModel  # noqa: E402


def test_native_fs_agent_registers_structured_tools() -> None:
    test_model = TestModel(call_tools=[])
    agent = create_native_fs_agent(test_model, "/tmp/ws", instructions="x")
    agent.run_sync("list tools", deps=NativeAgentDeps(workspace_root="/tmp/ws"))
    params = test_model.last_model_request_parameters
    assert params is not None
    registered = {tool.name for tool in params.function_tools}
    assert registered == set(NATIVE_TOOL_NAMES)


def test_path_guard_blocks_escape(tmp_path: Path) -> None:
    workspace = tmp_path / "ws"
    workspace.mkdir()
    with pytest.raises(NativePathError):
        validate_write_path(str(workspace), "../outside.txt")


@pytest.mark.parametrize(
    "path",
    ["bench/output/summary.md", "bench/output/status.json"],
)
def test_path_guard_allows_scenario_write_targets(tmp_path: Path, path: str) -> None:
    workspace = tmp_path / "ws"
    workspace.mkdir()
    resolved = validate_write_path(str(workspace), path)
    assert resolved.is_relative_to(workspace.resolve())


def test_mkdir_guard_only_allows_bench_output(tmp_path: Path) -> None:
    workspace = tmp_path / "ws"
    workspace.mkdir()
    target = validate_mkdir_path(str(workspace), "bench/output")
    target.mkdir(parents=True, exist_ok=True)
    assert target.is_dir()
    with pytest.raises(NativePathError):
        validate_mkdir_path(str(workspace), "other/dir")
