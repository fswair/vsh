from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class TailCommand(StructuredCommand):
    """Read the last lines from a file in the workspace."""

    _command_alias: ClassVar[str] = "tail"
    _boolean_flag_aliases: ClassVar[dict[str, str]] = {"follow": "f"}
    _flag_order: ClassVar[tuple[str, ...]] = ("follow",)
    _value_flag_aliases: ClassVar[dict[str, str]] = {"lines": "n"}
    _value_flag_order: ClassVar[tuple[str, ...]] = ("lines",)
    _positional_fields: ClassVar[tuple[str, ...]] = ("path",)

    kind: CommandKind = Field(
        default="read", description="Command category for partial file reads."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="read", risks=["reads file contents"])],
        description="Declared side effects for this command.",
    )
    path: str = Field(description="File path to read relative to the current working directory.")
    lines: int = Field(default=10, gt=0, description="Number of trailing lines to read.")
    follow: bool = Field(default=False, description="Follow appended data after the initial read.")
