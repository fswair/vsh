from __future__ import annotations as _annotations

from typing import ClassVar

import pytest

from vsh.schemas import (
    EchoCommand,
    FindCommand,
    SedCommand,
    SideEffect,
    StructuredCommand,
)
from vsh.schemas.common import CommandKind, SingleArgStyle, render_shell


class _ShellProbeCommand(StructuredCommand):
    _command_alias: ClassVar[str] = "probe"
    _single_arg_type: ClassVar[SingleArgStyle | None] = "individual"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"enabled": "e"}
    _flag_order: ClassVar[tuple[str, ...]] = ("missing", "enabled")
    _value_flag_aliases: ClassVar[dict[str, str]] = {"count": "c", "label": "l"}
    _value_flag_order: ClassVar[tuple[str, ...]] = ("missing_value", "count", "label")
    _positional_fields: ClassVar[tuple[str, ...]] = ("target",)

    kind: CommandKind = "read"
    side_effects: list[SideEffect] = []
    enabled: bool = True
    count: int | None = None
    label: str = ""
    target: str = "file.txt"


def test_render_shell_skips_unknown_aliases_and_empty_values() -> None:
    command = _ShellProbeCommand(enabled=True, count=3, label="", target="file.txt")

    assert render_shell(command) == "probe -e -c 3 file.txt"


def test_echo_to_shell_supports_no_newline_and_append() -> None:
    assert repr(EchoCommand(text="hi", no_newline=True)) == "echo -n hi"
    assert (
        repr(EchoCommand(text="line", output_path="notes.txt", append=True))
        == "echo line >> notes.txt"
    )


def test_find_to_shell_renders_all_filters() -> None:
    assert (
        repr(FindCommand(path=".", name="*.py", type="dir", maxdepth=2))
        == "find . -name *.py -type d -maxdepth 2"
    )


def test_sed_validator_requires_paths_and_supports_backup_suffix() -> None:
    with pytest.raises(ValueError, match="at least one sed path is required"):
        SedCommand(script="1p")

    command = SedCommand(
        script="s/a/b/g",
        path="file.txt",
        in_place=True,
        backup_suffix=".bak",
    )

    assert repr(command) == "sed -i .bak s/a/b/g file.txt"
    assert command.kind == "write"

    assert (
        repr(SedCommand(script="s/a/b/g", path="file.txt", in_place=True))
        == "sed -i s/a/b/g file.txt"
    )
    assert repr(SedCommand(script="1p", path="file.txt", quiet=False)) == "sed 1p file.txt"
