from __future__ import annotations as _annotations

import asyncio
from pathlib import Path
from unittest.mock import patch

import pytest
from click.testing import CliRunner

from vsh.cli import main
from vsh.mcp import (
    CODEMODE_INSTRUCTIONS,
    CODEMODE_SERVER_NAME,
    build_codemode_instructions,
    codemode_mcp,
    create_agent_codemode_server,
    create_codemode_server,
    load_custom_instructions,
    run_codemode_server,
)
from vsh.mcp.codemode_server import main as codemode_main
from vsh.mcp.prompts import register_codemode_prompts
from vsh.mcp.surface import register_vsh_surface


def test_create_codemode_server_registers_name_instructions_and_prompts() -> None:
    server = create_codemode_server()

    assert server.name == CODEMODE_SERVER_NAME
    assert server.instructions == CODEMODE_INSTRUCTIONS

    prompts = asyncio.run(server.list_prompts())
    prompt_names = {prompt.name for prompt in prompts}
    assert prompt_names == {
        "vsh_discover_command",
        "vsh_simulate_and_execute",
        "vsh_read_workspace",
    }


def test_create_agent_codemode_server_registers_minimal_tool_surface() -> None:
    server = create_agent_codemode_server()

    assert server.name == CODEMODE_SERVER_NAME
    assert server.instructions is None
    tools = asyncio.run(server.list_tools())
    tool_names = {tool.name for tool in tools}
    assert tool_names == {"apply", "apply_batch"}


def test_build_codemode_instructions_appends_custom_section() -> None:
    merged = build_codemode_instructions(custom_instructions="Only touch src/.")

    assert merged.startswith(CODEMODE_INSTRUCTIONS.rstrip())
    assert "Project-specific instructions:" in merged
    assert "Only touch src/." in merged


def test_build_codemode_instructions_ignores_blank_custom_text() -> None:
    assert build_codemode_instructions(custom_instructions="   \n") == CODEMODE_INSTRUCTIONS
    assert build_codemode_instructions(custom_instructions=None) == CODEMODE_INSTRUCTIONS


def test_create_codemode_server_accepts_custom_instructions() -> None:
    server = create_codemode_server(custom_instructions="Use vsh_rg for search.")

    instructions = server.instructions
    assert instructions is not None
    assert "Use vsh_rg for search." in instructions
    assert instructions.startswith(CODEMODE_INSTRUCTIONS.rstrip())


def test_load_custom_instructions_from_inline_and_file(tmp_path: Path) -> None:
    instructions_file = tmp_path / "extra.md"
    instructions_file.write_text("From file.\n", encoding="utf-8")

    loaded = load_custom_instructions(
        inline="From inline.",
        instructions_file=instructions_file,
    )

    assert loaded == "From file.\n\nFrom inline."


def test_load_custom_instructions_from_environment(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    instructions_file = tmp_path / "env.md"
    instructions_file.write_text("Env file text.", encoding="utf-8")
    monkeypatch.setenv("VSH_CODEMODE_INSTRUCTIONS_FILE", str(instructions_file))
    monkeypatch.setenv("VSH_CODEMODE_INSTRUCTIONS", "Env inline text.")

    loaded = load_custom_instructions()

    assert loaded == "Env file text.\n\nEnv inline text."


def test_load_custom_instructions_returns_none_when_unset() -> None:
    assert load_custom_instructions() is None


def test_codemode_module_exports_singleton() -> None:
    assert codemode_mcp.name == CODEMODE_SERVER_NAME


def test_register_helpers_are_idempotent_on_fresh_server() -> None:
    from fastmcp import FastMCP

    server = FastMCP("fresh-codemode-test")
    register_vsh_surface(server)
    register_codemode_prompts(server)

    prompts = asyncio.run(server.list_prompts())
    assert len(prompts) == 3


def test_cli_serve_codemode_starts_server() -> None:
    runner = CliRunner()

    with patch("vsh.mcp.codemode_server.run_codemode_server") as run:
        result = runner.invoke(main, ["serve-codemode"])

    assert result.exit_code == 0
    run.assert_called_once_with(inline=None, instructions_file=None)


def test_cli_serve_codemode_passes_custom_instructions(tmp_path: Path) -> None:
    runner = CliRunner()
    instructions_file = tmp_path / "rules.md"
    instructions_file.write_text("Repo rules.", encoding="utf-8")

    with patch("vsh.mcp.codemode_server.run_codemode_server") as run:
        result = runner.invoke(
            main,
            [
                "serve-codemode",
                "-i",
                "Inline rules.",
                "-f",
                str(instructions_file),
            ],
        )

    assert result.exit_code == 0
    run.assert_called_once_with(
        inline="Inline rules.",
        instructions_file=str(instructions_file),
    )


def test_run_codemode_server_uses_singleton_without_custom_instructions() -> None:
    with patch("vsh.mcp.codemode_server.codemode_mcp.run") as run:
        run_codemode_server()

    run.assert_called_once_with()


def test_run_codemode_server_builds_custom_server_when_instructions_present() -> None:
    with (
        patch("vsh.mcp.codemode_server.create_codemode_server") as create_server,
        patch("vsh.mcp.codemode_server.codemode_mcp.run") as default_run,
    ):
        custom_server = create_server.return_value
        run_codemode_server(inline="Custom only.")

    create_server.assert_called_once_with(custom_instructions="Custom only.")
    custom_server.run.assert_called_once_with()
    default_run.assert_not_called()


def test_codemode_main_entrypoint_starts_server() -> None:
    with patch("vsh.mcp.codemode_server.run_codemode_server") as run:
        codemode_main()

    run.assert_called_once_with()


async def _render_prompt(server, name: str) -> str:
    prompt = await server.get_prompt(name)
    result = await prompt.render()
    return result.messages[0].content.text


def test_codemode_prompts_return_workflow_guidance() -> None:
    server = create_codemode_server()

    discovery = asyncio.run(_render_prompt(server, "vsh_discover_command"))
    simulation = asyncio.run(_render_prompt(server, "vsh_simulate_and_execute"))
    read_only = asyncio.run(_render_prompt(server, "vsh_read_workspace"))

    assert "search" in discovery
    assert "get_schema" in discovery
    assert "snapshot_workspace" in simulation
    assert "execute_approved" in simulation
    assert "vsh_list" in read_only
    assert "workspace://projection/current" in read_only
