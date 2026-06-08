from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class RemoveCommand(StructuredCommand):
    """Delete files or directories from the workspace."""

    _command_alias: ClassVar[str] = "rm"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"recursive": "r", "force": "f"}
    _flag_order: ClassVar[tuple[str, ...]] = ("recursive", "force")
    _positional_fields: ClassVar[tuple[str, ...]] = ("path",)

    kind: CommandKind = Field(
        default="delete", description="Command category for destructive filesystem removal."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="delete", risks=["deletes filesystem nodes"])],
        description="Declared side effects for this command.",
    )
    path: str = Field(description="Path to remove relative to the current working directory.")
    recursive: bool = Field(
        default=False, description="Allow recursive deletion of directory trees."
    )
    force: bool = Field(
        default=False, description="Ignore missing targets and suppress interactive safeguards."
    )
