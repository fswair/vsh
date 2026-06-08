from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class StatCommand(StructuredCommand):
    """Read filesystem metadata for a path."""

    _command_alias: ClassVar[str] = "stat"
    _positional_fields: ClassVar[tuple[str, ...]] = ("path",)

    kind: CommandKind = Field(
        default="read", description="Command category for filesystem metadata reads."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="read", risks=["reads filesystem metadata"])],
        description="Declared side effects for this command.",
    )
    path: str = Field(description="Path whose metadata should be inspected.")
