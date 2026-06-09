from __future__ import annotations as _annotations

from pathlib import Path

from vsh.execute.dispatch import ExecutionContext, apply_command
from vsh.schemas import ApplyPatchCommand


def test_apply_patch_replaces_content(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "src.txt"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text("hello world\n", encoding="utf-8")
    command = ApplyPatchCommand(
        path="src.txt",
        patch="hello world\n===\nhello vsh\n",
        execution_reason="update greeting",
    )
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))
    effects = apply_command(command, ctx)
    assert target.read_text(encoding="utf-8").strip() == "hello vsh"
    assert effects.updates == [str(target.resolve())]
