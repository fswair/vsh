from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class ChmodCommand(StructuredCommand):
    """Change permission bits for a workspace path."""

    _command_alias: ClassVar[str] = "chmod"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"recursive": "R"}
    _flag_order: ClassVar[tuple[str, ...]] = ("recursive",)
    _positional_fields: ClassVar[tuple[str, ...]] = ("mode", "path")

    kind: CommandKind = Field(
        default="mutate", description="Command category for metadata mutation."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [
            SideEffect(kind="mutate", risks=["changes filesystem permissions"])
        ],
        description="Declared side effects for this command.",
    )
    mode: str = Field(description="Permission mode, for example 644, 755, or u+x.")
    path: str = Field(description="Path whose permissions should change.")
    recursive: bool = Field(
        default=False, description="Apply permissions recursively to directory trees."
    )
