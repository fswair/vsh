from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class SortCommand(StructuredCommand):
    """Read and sort file lines in simulation."""

    _command_alias: ClassVar[str] = "sort"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"unique": "u", "reverse": "r"}
    _flag_order: ClassVar[tuple[str, ...]] = ("unique", "reverse")
    _positional_fields: ClassVar[tuple[str, ...]] = ("path",)

    kind: CommandKind = Field(
        default="read", description="Command category for file content reads."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="read", risks=["reads file contents"])],
        description="Declared side effects for this command.",
    )
    path: str = Field(description="File path to sort relative to the current working directory.")
    unique: bool = Field(default=False, description="Emit only unique lines.")
    reverse: bool = Field(default=False, description="Sort in reverse order.")
