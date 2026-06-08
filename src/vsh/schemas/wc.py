from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, SingleArgStyle, StructuredCommand


class WcCommand(StructuredCommand):
    """Count lines, words, bytes, or characters in a file."""

    _command_alias: ClassVar[str] = "wc"
    _single_arg_type: ClassVar[SingleArgStyle | None] = "concatenate"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {
        "lines": "l",
        "words": "w",
        "bytes": "c",
        "chars": "m",
    }
    _flag_order: ClassVar[tuple[str, ...]] = ("lines", "words", "bytes", "chars")
    _positional_fields: ClassVar[tuple[str, ...]] = ("path",)

    kind: CommandKind = Field(
        default="read", description="Command category for file content reads."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="read", risks=["reads file contents"])],
        description="Declared side effects for this command.",
    )
    path: str = Field(description="File path to count relative to the current working directory.")
    lines: bool = Field(default=False, description="Count newline characters.")
    words: bool = Field(default=False, description="Count words.")
    bytes: bool = Field(default=False, description="Count bytes.")
    chars: bool = Field(default=False, description="Count characters.")
