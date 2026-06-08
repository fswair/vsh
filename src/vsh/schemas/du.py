from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, SingleArgStyle, StructuredCommand


class DuCommand(StructuredCommand):
    """Estimate disk usage from workspace metadata."""

    _command_alias: ClassVar[str] = "du"
    _single_arg_type: ClassVar[SingleArgStyle | None] = "concatenate"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"summarize": "s", "human_readable": "h"}
    _flag_order: ClassVar[tuple[str, ...]] = ("summarize", "human_readable")
    _positional_fields: ClassVar[tuple[str, ...]] = ("path",)

    kind: CommandKind = Field(
        default="read", description="Command category for filesystem metadata reads."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="read", risks=["reads filesystem metadata"])],
        description="Declared side effects for this command.",
    )
    path: str = Field(default=".", description="Path whose disk usage should be estimated.")
    summarize: bool = Field(default=False, description="Display only a total for each path.")
    human_readable: bool = Field(default=False, description="Render sizes in human-readable units.")
