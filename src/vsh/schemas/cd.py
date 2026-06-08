from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class CdCommand(StructuredCommand):
    """Change the session working directory inside the active workspace."""

    _command_alias: ClassVar[str] = "cd"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"physical": "P"}
    _flag_order: ClassVar[tuple[str, ...]] = ("physical",)
    _positional_fields: ClassVar[tuple[str, ...]] = ("path",)

    kind: CommandKind = Field(
        default="read", description="Command category for workspace navigation."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [
            SideEffect(kind="read", risks=["path may resolve outside the workspace"]),
            SideEffect(kind="mutate", risks=["changes session cwd"]),
        ],
        description="Declared side effects for this command.",
    )
    path: str = Field(
        description="Target directory path relative to the current working directory or absolute."
    )
    physical: bool = Field(
        default=False,
        description="Resolve and display the physical path instead of the logical session path.",
    )
