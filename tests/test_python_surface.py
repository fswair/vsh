from __future__ import annotations

import asyncio
import importlib
import json
import runpy
import sys
from pathlib import Path
from typing import Literal, cast

import pytest
from fastmcp import Client, FastMCP
from fastmcp.exceptions import ToolError
from fastmcp.prompts import PromptResult

from vsh import __version__, cli
from vsh._response import bound_mcp_response, response_size
from vsh.mcp import codemode_server
from vsh.mcp.native_tools import DetailName, PolicyName, RunModeName, vsh_run
from vsh.mcp.surface import register_vsh_surface


def test_mcp_authority_is_fixed_by_host_not_tool_arguments(tmp_path, monkeypatch) -> None:
    authorized = tmp_path / "authorized"
    outside = tmp_path / "outside"
    authorized.mkdir()
    outside.mkdir()
    (authorized / "visible.txt").write_text("host workspace")
    server = FastMCP("fixed-root-test")
    register_vsh_surface(
        server, workspace_root=str(authorized), policy="strict", budget={"max_program_bytes": 512}
    )
    monkeypatch.chdir(outside)

    async def exercise() -> None:
        async with Client(server) as client:
            tool = (await client.list_tools())[0]
            assert (
                not {"workspace_root", "policy", "budget"} & tool.inputSchema["properties"].keys()
            )
            result = await client.call_tool(
                "vsh_run", {"code": "vsh_read('/workspace/visible.txt')"}
            )
            assert result.data["result_repr"] == "'host workspace'"
            for argument, value in {
                "workspace_root": str(outside),
                "policy": "balanced",
                "budget": {"max_program_bytes": 99999},
            }.items():
                with pytest.raises(ToolError):
                    await client.call_tool("vsh_run", {"code": "None", argument: value})

    asyncio.run(exercise())
    assert not (outside / ".vsh-runtime").exists()


def test_mcp_response_has_one_aggregate_json_budget(tmp_path) -> None:
    server = FastMCP("bounded-response-test")
    register_vsh_surface(server, workspace_root=str(tmp_path), max_response_bytes=4096)

    async def exercise() -> None:
        async with Client(server) as client:
            result = await client.call_tool(
                "vsh_run", {"code": "print('🙂' * 10000)\n'🙂' * 10000", "mode": "auto"}
            )
            assert result.data["state"] == "committed"
            assert result.data["response_truncated"]
            assert len(json.dumps(result.data, ensure_ascii=True, separators=(",", ":"))) <= 4096

    asyncio.run(exercise())


def test_response_budget_omits_full_changes_and_denial_text_but_keeps_outcome() -> None:
    payload: dict[str, object] = {
        "state": "denied",
        "commit": {"committed": False},
        "changes": [{"path": "🙂" * 2000, "kind": "create"}],
        "deny_reason": "not authorized " * 2000,
        "bash": {"stdout": "x" * 10000},
    }
    result = bound_mcp_response(payload, 4096)
    assert response_size(result) <= 4096
    assert result["state"] == "denied" and result["commit"] == {"committed": False}
    assert result["changes"] == [] and result["deny_reason"] is None
    assert result["response_truncated"] is True


def test_mcp_host_configuration_is_validated(tmp_path) -> None:
    file = tmp_path / "file.txt"
    file.write_text("not a directory")
    with pytest.raises(NotADirectoryError):
        register_vsh_surface(FastMCP("invalid-root"), workspace_root=str(file))
    with pytest.raises(ValueError, match="unknown policy"):
        register_vsh_surface(FastMCP("invalid-policy"), policy=cast(PolicyName, "invalid"))
    with pytest.raises(ValueError, match="max_response_bytes"):
        register_vsh_surface(FastMCP("invalid-size"), max_response_bytes=1)


def test_mcp_bash_response_has_one_byte_authoritative_copy(tmp_path, monkeypatch) -> None:
    monkeypatch.setenv("VSH_ENABLE_BASH", "1")
    server = FastMCP("bash-response-test")
    register_vsh_surface(server, workspace_root=str(tmp_path))

    async def exercise() -> None:
        async with Client(server) as client:
            result = await client.call_tool("vsh_run", {"code": "printf hello", "language": "bash"})
            assert result.data["bash"]["stdout"] == "aGVsbG8="
            assert (
                result.data["stdout"] == result.data["stderr"] == result.data["result_repr"] == ""
            )

    asyncio.run(exercise())


def test_mcp_completed_read_previews_do_not_exhaust_retention(tmp_path) -> None:
    server = FastMCP("preview-cleanup-test")
    register_vsh_surface(server, workspace_root=str(tmp_path))

    async def exercise() -> None:
        async with Client(server) as client:
            for _ in range(66):
                result = await client.call_tool("vsh_run", {"code": "42"})
                assert result.data["result_repr"] == "42"
                assert result.data["preview_retained"] is False
            changed = await client.call_tool(
                "vsh_run", {"code": "vsh_write('/workspace/new.txt', 'pending')"}
            )
            assert changed.data["preview_retained"] is True
            assert not (tmp_path / "new.txt").exists()

    asyncio.run(exercise())


def test_cli_reports_version(capsys: pytest.CaptureFixture[str]) -> None:
    with pytest.raises(SystemExit, match="0"):
        cli.main(["--version"])

    assert capsys.readouterr().out == f"{__version__}\n"


def test_cli_runs_inline_code_and_emits_compact_json(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    cli.main(["run", "--code", "40 + 2", "--workspace", str(tmp_path)])

    payload = json.loads(capsys.readouterr().out)
    assert payload["state"] == "auto_approved"
    assert payload["result_repr"] == "42"


def test_cli_reads_a_file_and_auto_commits(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    program = tmp_path / "program.py"
    program.write_text(
        "from pathlib import Path\nPath('/workspace/from-cli.txt').write_text('cli')",
        encoding="utf-8",
    )

    cli.main(
        [
            "run",
            "--file",
            str(program),
            "--workspace",
            str(tmp_path),
            "--intent",
            "exercise the native CLI",
            "--mode",
            "auto",
            "--policy",
            "balanced",
            "--detail",
            "full",
        ]
    )

    payload = json.loads(capsys.readouterr().out)
    assert payload["state"] == "committed"
    assert (tmp_path / "from-cli.txt").read_text(encoding="utf-8") == "cli"


def test_cli_promotes_an_exact_preview_transaction(
    tmp_path: Path,
    capsys: pytest.CaptureFixture[str],
) -> None:
    preview = vsh_run(
        "from pathlib import Path\nPath('/workspace/promoted.txt').write_text('yes')",
        workspace_root=str(tmp_path),
    )

    cli.main(
        [
            "run",
            "--transaction",
            str(preview["transaction"]),
            "--workspace",
            str(tmp_path),
            "--mode",
            "auto",
        ]
    )

    payload = json.loads(capsys.readouterr().out)
    assert payload["state"] == "committed"
    assert (tmp_path / "promoted.txt").read_text(encoding="utf-8") == "yes"


def test_cli_serve_uses_the_single_mcp_server(monkeypatch: pytest.MonkeyPatch) -> None:
    from vsh.mcp.server import mcp

    calls: list[str] = []
    monkeypatch.setattr(mcp, "run", lambda: calls.append("run"))

    cli.main(["serve"])

    assert calls == ["run"]


def test_python_module_entrypoint_delegates_to_cli(monkeypatch: pytest.MonkeyPatch) -> None:
    importlib.import_module("vsh.__main__")
    sys.modules.pop("vsh.__main__", None)
    calls: list[str] = []
    monkeypatch.setattr(cli, "main", lambda: calls.append("main"))

    runpy.run_module("vsh.__main__", run_name="__main__")

    assert calls == ["main"]


@pytest.mark.parametrize(
    ("option", "message"),
    [
        ("mode", "unknown run mode"),
        ("detail", "unknown receipt detail"),
        ("policy", "unknown policy profile"),
    ],
)
def test_vsh_run_rejects_unknown_public_options(
    tmp_path: Path,
    option: Literal["mode", "detail", "policy"],
    message: str,
) -> None:
    # Casts deliberately cross the typed API boundary to verify runtime validation.
    with pytest.raises(ValueError, match=message):
        if option == "mode":
            vsh_run(
                "1",
                workspace_root=str(tmp_path),
                mode=cast(RunModeName, "invalid"),
            )
        elif option == "detail":
            vsh_run(
                "1",
                workspace_root=str(tmp_path),
                detail=cast(DetailName, "invalid"),
            )
        else:
            vsh_run(
                "1",
                workspace_root=str(tmp_path),
                policy=cast(PolicyName, "invalid"),
            )


def test_vsh_run_validates_workspace_and_transaction_shapes(tmp_path: Path) -> None:
    not_a_directory = tmp_path / "file.txt"
    not_a_directory.write_text("x", encoding="utf-8")

    with pytest.raises(NotADirectoryError):
        vsh_run("1", workspace_root=str(not_a_directory))
    with pytest.raises(ValueError, match="either code or a preview transaction"):
        vsh_run("1", transaction="tx", workspace_root=str(tmp_path), mode="auto")
    with pytest.raises(ValueError, match="only be resumed"):
        vsh_run(transaction="tx", workspace_root=str(tmp_path))
    with pytest.raises(ValueError, match="code is required"):
        vsh_run(workspace_root=str(tmp_path))


def test_vsh_run_uses_default_workspace_and_budget_overrides(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.chdir(tmp_path)

    payload = vsh_run("40 + 2", budget={"max_program_bytes": 1024})

    assert payload["result_repr"] == "42"


@pytest.mark.parametrize("character", ["x", "é", "🙂"])
def test_vsh_run_bounds_large_inline_result_and_stdout(tmp_path: Path, character: str) -> None:
    payload = vsh_run(
        f"text = {character!r} * 70000\nprint(text)\ntext",
        workspace_root=str(tmp_path),
        budget={
            "max_output_bytes": 400_000,
            "max_result_bytes": 400_000,
        },
    )

    assert payload["result_truncated"] is True
    assert payload["stdout_truncated"] is True
    assert str(payload["result_repr"]).endswith("…")
    assert str(payload["stdout"]).endswith("…")
    assert len(str(payload["result_repr"])) == 65_537
    assert len(str(payload["stdout"])) == 65_537
    assert str(payload["stdout"]) == character * 65_536 + "…"


def test_codemode_instruction_composition() -> None:
    assert codemode_server.build_codemode_instructions() == codemode_server.CODEMODE_INSTRUCTIONS
    assert (
        codemode_server.build_codemode_instructions(custom_instructions="  \n")
        == codemode_server.CODEMODE_INSTRUCTIONS
    )
    for name in (
        "vsh_read",
        "vsh_write",
        "vsh_list",
        "vsh_mkdir",
        "vsh_remove",
        "vsh_move",
        "vsh_copy",
        "vsh_glob",
        "vsh_search",
        "vsh_patch",
    ):
        assert f"`{name}`" in codemode_server.CODEMODE_INSTRUCTIONS
    assert "same active overlay" in codemode_server.CODEMODE_INSTRUCTIONS

    merged = codemode_server.build_codemode_instructions(
        custom_instructions="  Keep receipts compact.  "
    )
    assert "Project-specific instructions:" in merged
    assert merged.endswith("Keep receipts compact.\n")


def test_codemode_instruction_sources(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    instructions = tmp_path / "instructions.md"
    instructions.write_text("  from file  ", encoding="utf-8")

    assert (
        codemode_server.load_custom_instructions(
            inline="  inline  ", instructions_file=instructions
        )
        == "from file\n\ninline"
    )

    monkeypatch.setenv("VSH_CODEMODE_INSTRUCTIONS_FILE", str(instructions))
    monkeypatch.setenv("VSH_CODEMODE_INSTRUCTIONS", "  from env  ")
    assert codemode_server.load_custom_instructions() == "from file\n\nfrom env"

    monkeypatch.delenv("VSH_CODEMODE_INSTRUCTIONS_FILE")
    monkeypatch.delenv("VSH_CODEMODE_INSTRUCTIONS")
    assert codemode_server.load_custom_instructions(inline="  ") is None
    assert codemode_server.load_custom_instructions() is None


async def _assert_codemode_server_contracts() -> None:
    server = codemode_server.create_codemode_server(custom_instructions="project rule")
    tools = await server.list_tools()
    prompts = await server.list_prompts()
    prompt = await server.get_prompt("vsh_run_transaction")
    assert prompt is not None
    rendered = await prompt.render()
    assert isinstance(rendered, PromptResult)

    assert [tool.name for tool in tools] == ["vsh_run"]
    assert [item.name for item in prompts] == ["vsh_run_transaction"]
    assert "single `vsh_run` tool" in str(rendered.messages[0].content)
    assert "`vsh_search`" in str(rendered.messages[0].content)


def test_codemode_server_exposes_only_the_native_surface() -> None:
    asyncio.run(_assert_codemode_server_contracts())


class _RunRecorder:
    def __init__(self) -> None:
        self.calls = 0

    def run(self) -> None:
        self.calls += 1


def test_codemode_runner_reuses_default_or_builds_custom_server(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    default = _RunRecorder()
    custom = _RunRecorder()
    custom_values: list[str | None] = []
    monkeypatch.setattr(codemode_server.codemode_mcp, "run", default.run)

    codemode_server.run_codemode_server()

    def create_custom(*, custom_instructions: str | None = None) -> _RunRecorder:
        custom_values.append(custom_instructions)
        return custom

    monkeypatch.setattr(codemode_server, "create_codemode_server", create_custom)
    codemode_server.run_codemode_server(inline="custom")

    assert default.calls == 1
    assert custom.calls == 1
    assert custom_values == ["custom"]


def test_codemode_main_delegates_to_runner(monkeypatch: pytest.MonkeyPatch) -> None:
    calls: list[str] = []
    monkeypatch.setattr(
        codemode_server,
        "run_codemode_server",
        lambda: calls.append("run"),
    )

    codemode_server.main()

    assert calls == ["run"]
