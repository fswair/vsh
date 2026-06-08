from __future__ import annotations as _annotations

import json
from unittest.mock import patch

from click.testing import CliRunner

from vsh.cli import main


def test_cli_search_emits_command_specs() -> None:
    runner = CliRunner()
    result = runner.invoke(main, ["search", "ls"])

    assert result.exit_code == 0
    payload = json.loads(result.output)
    assert payload[0]["name"] == "vsh_list"


def test_cli_schema_emits_json_schema() -> None:
    runner = CliRunner()
    result = runner.invoke(main, ["schema", "vsh_pwd"])

    assert result.exit_code == 0
    schema = json.loads(result.output)
    assert schema["title"] == "PwdCommand"


def test_cli_names_emits_command_names() -> None:
    runner = CliRunner()
    result = runner.invoke(main, ["names", "vsh_grep"])

    assert result.exit_code == 0
    assert json.loads(result.output) == ["vsh_grep"]


def test_cli_serve_starts_mcp_server() -> None:
    runner = CliRunner()

    with patch("vsh.mcp.server.mcp.run") as run:
        result = runner.invoke(main, ["serve"])

    assert result.exit_code == 0
    run.assert_called_once_with()
