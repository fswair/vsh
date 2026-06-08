from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, SingleArgStyle, StructuredCommand


class GrepCommand(StructuredCommand):
    """Search file contents for lines matching a pattern."""

    _command_alias: ClassVar[str] = "grep"
    _single_arg_type: ClassVar[SingleArgStyle | None] = "concatenate"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {
        "ignore_case": "i",
        "line_number": "n",
        "recursive": "r",
        "fixed_strings": "F",
        "extended_regexp": "E",
    }
    _flag_order: ClassVar[tuple[str, ...]] = (
        "ignore_case",
        "line_number",
        "recursive",
        "fixed_strings",
        "extended_regexp",
    )
    _positional_fields: ClassVar[tuple[str, ...]] = ("pattern", "path")

    kind: CommandKind = Field(default="search", description="Command category for content search.")
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="search", risks=["reads file contents"])],
        description="Declared side effects for this command.",
    )
    pattern: str = Field(description="Pattern to search for.")
    path: str = Field(default=".", description="File or directory path to search.")
    ignore_case: bool = Field(default=False, description="Match without case sensitivity.")
    line_number: bool = Field(default=False, description="Prefix matches with line numbers.")
    recursive: bool = Field(default=False, description="Search directories recursively.")
    fixed_strings: bool = Field(default=False, description="Treat the pattern as a fixed string.")
    extended_regexp: bool = Field(default=False, description="Use extended regular expressions.")
