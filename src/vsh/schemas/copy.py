from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class CopyCommand(StructuredCommand):
    """Copy files or directories inside the workspace."""

    _command_alias: ClassVar[str] = "cp"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"recursive": "r", "overwrite": "f"}
    _flag_order: ClassVar[tuple[str, ...]] = ("recursive", "overwrite")
    _positional_fields: ClassVar[tuple[str, ...]] = ("src", "dst")

    kind: CommandKind = Field(default="copy", description="Command category for filesystem copies.")
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [
            SideEffect(kind="read", risks=["reads source metadata or content"]),
            SideEffect(kind="copy", risks=["creates destination nodes"]),
        ],
        description="Declared side effects for this command.",
    )
    src: str = Field(description="Source path to copy relative to the current working directory.")
    dst: str = Field(
        description="Destination path to create relative to the current working directory."
    )
    recursive: bool = Field(default=False, description="Copy directories recursively.")
    overwrite: bool = Field(
        default=False, description="Allow replacement when the destination already exists."
    )
