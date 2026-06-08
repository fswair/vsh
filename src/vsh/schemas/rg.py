from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, SingleArgStyle, StructuredCommand


class RgCommand(StructuredCommand):
    """Search workspace contents with ripgrep-style options."""

    _command_alias: ClassVar[str] = "rg"
    _single_arg_type: ClassVar[SingleArgStyle | None] = "concatenate"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {
        "ignore_case": "i",
        "line_number": "n",
        "fixed_strings": "F",
        "hidden": "hidden",
    }
    _flag_order: ClassVar[tuple[str, ...]] = (
        "ignore_case",
        "line_number",
        "fixed_strings",
        "hidden",
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
    fixed_strings: bool = Field(default=False, description="Treat the pattern as a fixed string.")
    hidden: bool = Field(default=False, description="Include hidden files and directories.")
