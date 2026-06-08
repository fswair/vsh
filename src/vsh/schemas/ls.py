from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, SingleArgStyle, StructuredCommand


class LsCommand(StructuredCommand):
    """List directory entries from the current workspace snapshot."""

    _command_alias: ClassVar[str] = "ls"
    _single_arg_type: ClassVar[SingleArgStyle | None] = "concatenate"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {
        "long": "l",
        "all": "a",
        "one": "1",
        "recursive": "R",
    }
    _flag_order: ClassVar[tuple[str, ...]] = ("long", "all", "one", "recursive")
    _positional_fields: ClassVar[tuple[str, ...]] = ("path",)

    kind: CommandKind = Field(default="list", description="Command category for directory listing.")
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="list", risks=[])],
        description="Declared side effects for this command.",
    )
    path: str = Field(
        default=".", description="Directory path to list relative to the current working directory."
    )
    all: bool = Field(default=False, description="Include entries whose names start with a dot.")
    long: bool = Field(default=False, description="Render the long listing format.")
    one: bool = Field(default=False, description="Render one entry per output line.")
    recursive: bool = Field(
        default=False, description="Descend into nested directories recursively."
    )
