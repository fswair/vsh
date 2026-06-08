from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class NlCommand(StructuredCommand):
    """Read a file with line numbers."""

    _command_alias: ClassVar[str] = "nl"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"number_all": "ba"}
    _flag_order: ClassVar[tuple[str, ...]] = ("number_all",)
    _positional_fields: ClassVar[tuple[str, ...]] = ("path",)

    kind: CommandKind = Field(
        default="read", description="Command category for file content reads."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="read", risks=["reads file contents"])],
        description="Declared side effects for this command.",
    )
    path: str = Field(description="File path to read relative to the current working directory.")
    number_all: bool = Field(default=False, description="Number all lines, including blank lines.")
