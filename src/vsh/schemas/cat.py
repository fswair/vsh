from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, SingleArgStyle, StructuredCommand


class CatCommand(StructuredCommand):
    """Read file contents from the workspace snapshot."""

    _command_alias: ClassVar[str] = "cat"
    _single_arg_type: ClassVar[SingleArgStyle | None] = "concatenate"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {
        "number": "n",
        "squeeze_blank": "s",
        "show_ends": "E",
    }
    _flag_order: ClassVar[tuple[str, ...]] = ("number", "squeeze_blank", "show_ends")
    _positional_fields: ClassVar[tuple[str, ...]] = ("path",)

    kind: CommandKind = Field(
        default="read", description="Command category for file content reads."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="read", risks=["reads file contents"])],
        description="Declared side effects for this command.",
    )
    path: str = Field(description="File path to read relative to the current working directory.")
    number: bool = Field(default=False, description="Number all output lines.")
    squeeze_blank: bool = Field(default=False, description="Suppress repeated empty output lines.")
    show_ends: bool = Field(default=False, description="Mark line endings in rendered output.")
