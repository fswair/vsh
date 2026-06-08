from __future__ import annotations as _annotations

import pytest
from pydantic import ValidationError

from vsh import (
    CatCommand,
    CdCommand,
    ChmodCommand,
    DuCommand,
    EchoCommand,
    FindCommand,
    GrepCommand,
    HeadCommand,
    LnCommand,
    LsCommand,
    PwdCommand,
    RgCommand,
    SedCommand,
    StatCommand,
    StructuredCommand,
    TailCommand,
    WcCommand,
    get_schema,
    registry,
    search,
)
from vsh.mcp import resources, tools
from vsh.registry import search_names


def test_registry_contains_initial_command_surface() -> None:
    assert set(registry) >= {
        "vsh_pwd",
        "vsh_cd",
        "vsh_list",
        "vsh_cat",
        "vsh_head",
        "vsh_tail",
        "vsh_grep",
        "vsh_rg",
        "vsh_find",
        "vsh_wc",
        "vsh_echo",
        "vsh_sed",
        "vsh_nl",
        "vsh_stat",
        "vsh_du",
        "vsh_chmod",
        "vsh_link",
        "vsh_mkdir",
        "vsh_touch",
        "vsh_move",
        "vsh_copy",
        "vsh_remove",
        "vsh_curl",
        "vsh_wget",
    }


def test_search_returns_command_specs() -> None:
    results = search("ls")

    assert [spec.name for spec in results] == ["vsh_list"]
    assert results[0].schema_model_name == "LsCommand"


def test_search_names_preserves_phase_one_acceptance_criterion() -> None:
    assert search_names("ls") == ["vsh_list"]


def test_get_schema_returns_structured_command_json_schema() -> None:
    schema = get_schema("vsh_list")

    assert schema["title"] == "LsCommand"
    assert schema["type"] == "object"
    assert schema["properties"]["path"]["default"] == "."
    assert schema["properties"]["path"]["description"] == (
        "Directory path to list relative to the current working directory."
    )
    assert schema["properties"]["long"]["description"] == "Render the long listing format."
    assert schema["properties"]["raw_command"]["description"].startswith(
        "Optional raw shell command"
    )
    assert "long" in schema["properties"]
    assert "l" not in schema["properties"]
    assert "side_effects" in schema["properties"]


def test_get_schema_unknown_command_raises_key_error() -> None:
    with pytest.raises(KeyError, match="unknown command spec"):
        get_schema("missing")


def test_command_models_are_structured_commands() -> None:
    assert isinstance(PwdCommand(), StructuredCommand)
    assert isinstance(CdCommand(path="src"), StructuredCommand)
    assert isinstance(LsCommand(path="src", all=True), StructuredCommand)
    assert isinstance(CatCommand(path="README.md"), StructuredCommand)
    assert isinstance(GrepCommand(pattern="TODO", path="src"), StructuredCommand)
    assert isinstance(RgCommand(pattern="TODO"), StructuredCommand)
    assert isinstance(FindCommand(path=".", name="*.py"), StructuredCommand)
    assert isinstance(EchoCommand(text="hello", output_path="hello.txt"), StructuredCommand)
    assert isinstance(SedCommand(script="1,20p", path="README.md"), StructuredCommand)
    assert isinstance(
        SedCommand(script="s/a/b/g", paths=["a.txt", "b.txt"], in_place=True), StructuredCommand
    )
    assert isinstance(StatCommand(path="README.md"), StructuredCommand)
    assert isinstance(DuCommand(path="src", summarize=True), StructuredCommand)
    assert isinstance(ChmodCommand(mode="+x", path="script.sh"), StructuredCommand)
    assert isinstance(LnCommand(src="src", dst="current-src", symbolic=True), StructuredCommand)
    assert LsCommand(path="src", long=True).long is True


def test_list_command_renders_shell_preview_with_concatenated_flags() -> None:
    command = LsCommand(path=".", all=True, long=True)

    assert command.kind == "list"
    assert command.side_effects[0].kind == "list"
    assert repr(command) == "ls -la ."


def test_list_command_can_compare_optional_raw_command_to_preview() -> None:
    command = LsCommand(path=".", all=True, long=True, raw_command="ls -la .")

    assert command.raw_matches_shell_preview() is True


def test_common_agent_commands_render_shell_previews() -> None:
    assert repr(CatCommand(path="README.md", number=True)) == "cat -n README.md"
    assert repr(HeadCommand(path="README.md", lines=5)) == "head -n 5 README.md"
    assert repr(TailCommand(path="app.log", lines=20, follow=True)) == "tail -f -n 20 app.log"
    assert (
        repr(
            GrepCommand(
                pattern="TODO",
                path="src",
                ignore_case=True,
                line_number=True,
                recursive=True,
            )
        )
        == "grep -inr TODO src"
    )
    assert (
        repr(RgCommand(pattern="TODO", path="src", ignore_case=True, line_number=True))
        == "rg -in TODO src"
    )
    assert (
        repr(FindCommand(path=".", name="*.py", type="file", maxdepth=2))
        == "find . -name '*.py' -type f -maxdepth 2"
    )
    assert repr(WcCommand(path="README.md", lines=True, words=True)) == "wc -lw README.md"
    assert (
        repr(EchoCommand(text="hello world", output_path="notes.txt"))
        == "echo 'hello world' > notes.txt"
    )
    assert repr(SedCommand(script="1,20p", path="README.md")) == "sed -n 1,20p README.md"
    assert (
        repr(SedCommand(script="s/a/b/g", paths=["a.txt", "b.txt"], in_place=True))
        == "sed -i s/a/b/g a.txt b.txt"
    )
    assert repr(StatCommand(path="README.md")) == "stat README.md"
    assert repr(DuCommand(path="src", summarize=True, human_readable=True)) == "du -sh src"
    assert repr(ChmodCommand(mode="755", path="scripts", recursive=True)) == "chmod -R 755 scripts"
    assert (
        repr(LnCommand(src="src", dst="current-src", symbolic=True, force=True))
        == "ln -sf src current-src"
    )


def test_command_models_forbid_extra_fields() -> None:
    with pytest.raises(ValidationError):
        LsCommand.model_validate({"path": ".", "unexpected": True})


def test_mcp_tools_expose_plan_tool_surface_without_server_import() -> None:
    assert [spec.name for spec in tools.search("ls")] == ["vsh_list"]
    assert tools.get_schema("vsh_pwd")["title"] == "PwdCommand"


def test_command_spec_resource_returns_real_schema() -> None:
    payload = resources.command_spec("vsh_list")

    assert "long" in payload["schema"]["properties"]
    assert "l" not in payload["schema"]["properties"]
