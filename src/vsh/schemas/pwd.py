from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class PwdCommand(StructuredCommand):
    """Return the current working directory tracked by the session."""

    _command_alias: ClassVar[str] = "pwd"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"physical": "P"}
    _flag_order: ClassVar[tuple[str, ...]] = ("physical",)

    kind: CommandKind = Field(
        default="read", description="Command category for current-directory reads."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="read", risks=[])],
        description="Declared side effects for this command.",
    )
    physical: bool = Field(
        default=False,
        description="Render the physical filesystem path instead of the logical session path.",
    )
