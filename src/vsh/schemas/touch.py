from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class TouchCommand(StructuredCommand):
    """Create a file or update its timestamp in the workspace projection."""

    _command_alias: ClassVar[str] = "touch"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"no_create": "c"}
    _flag_order: ClassVar[tuple[str, ...]] = ("no_create",)
    _positional_fields: ClassVar[tuple[str, ...]] = ("path",)

    kind: CommandKind = Field(
        default="write", description="Command category for file writes or updates."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="write", risks=["creates or mutates a file"])],
        description="Declared side effects for this command.",
    )
    path: str = Field(description="File path to touch relative to the current working directory.")
    no_create: bool = Field(
        default=False, description="Update only existing files and skip missing targets."
    )
