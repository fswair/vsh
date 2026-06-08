from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, SingleArgStyle, StructuredCommand


class LnCommand(StructuredCommand):
    """Create a hard link or symbolic link inside the workspace."""

    _command_alias: ClassVar[str] = "ln"
    _single_arg_type: ClassVar[SingleArgStyle | None] = "concatenate"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"symbolic": "s", "force": "f"}
    _flag_order: ClassVar[tuple[str, ...]] = ("symbolic", "force")
    _positional_fields: ClassVar[tuple[str, ...]] = ("src", "dst")

    kind: CommandKind = Field(default="create", description="Command category for link creation.")
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [
            SideEffect(kind="create", risks=["creates filesystem link nodes"])
        ],
        description="Declared side effects for this command.",
    )
    src: str = Field(description="Existing source path for the link target.")
    dst: str = Field(description="New link path to create.")
    symbolic: bool = Field(
        default=False, description="Create a symbolic link instead of a hard link."
    )
    force: bool = Field(
        default=False, description="Replace an existing destination link when possible."
    )
