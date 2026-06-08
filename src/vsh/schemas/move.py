from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class MoveCommand(StructuredCommand):
    """Move or rename a filesystem node inside the workspace."""

    _command_alias: ClassVar[str] = "mv"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"overwrite": "f"}
    _flag_order: ClassVar[tuple[str, ...]] = ("overwrite",)
    _positional_fields: ClassVar[tuple[str, ...]] = ("src", "dst")

    kind: CommandKind = Field(default="move", description="Command category for renames and moves.")
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [
            SideEffect(kind="move", risks=["source path changes location"]),
            SideEffect(kind="write", risks=["destination may be overwritten"]),
        ],
        description="Declared side effects for this command.",
    )
    src: str = Field(description="Source path to move relative to the current working directory.")
    dst: str = Field(
        description="Destination path to write relative to the current working directory."
    )
    overwrite: bool = Field(
        default=False, description="Allow replacement when the destination already exists."
    )
