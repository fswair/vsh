from __future__ import annotations as _annotations

import shlex
from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class EchoCommand(StructuredCommand):
    """Render text to stdout or predict writing it to a file."""

    _command_alias: ClassVar[str] = "echo"

    kind: CommandKind = Field(
        default="write", description="Command category for text output or file writes."
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="write", risks=["may write text to a file"])],
        description="Declared side effects for this command.",
    )
    text: str = Field(description="Text to emit.")
    output_path: str | None = Field(
        default=None, description="Optional file path to write the text into."
    )
    append: bool = Field(
        default=False, description="Append to the output file instead of replacing it."
    )
    no_newline: bool = Field(default=False, description="Do not print the trailing newline.")

    def to_shell(self) -> str:
        tokens = [self._command_alias]
        if self.no_newline:
            tokens.append("-n")
        tokens.append(shlex.quote(self.text))
        if self.output_path:
            tokens.append(">>" if self.append else ">")
            tokens.append(shlex.quote(self.output_path))
        return " ".join(tokens)
