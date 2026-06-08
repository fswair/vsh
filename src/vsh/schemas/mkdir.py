from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class MkdirCommand(StructuredCommand):
    """Create one directory node inside the workspace projection."""

    _command_alias: ClassVar[str] = "mkdir"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"parents": "p"}
    _flag_order: ClassVar[tuple[str, ...]] = ("parents",)
    _positional_fields: ClassVar[tuple[str, ...]] = ("path",)

    kind: CommandKind = Field(
        default="create", description="Command category for filesystem creation."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="create", risks=["creates filesystem nodes"])],
        description="Declared side effects for this command.",
    )
    path: str = Field(
        description="Directory path to create relative to the current working directory."
    )
    parents: bool = Field(
        default=False, description="Create missing parent directories when they do not exist."
    )
