from __future__ import annotations as _annotations

from typing import ClassVar, Literal

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class FindCommand(StructuredCommand):
    """Search workspace paths by metadata."""

    _command_alias: ClassVar[str] = "find"

    kind: CommandKind = Field(default="search", description="Command category for metadata search.")
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="search", risks=["reads filesystem metadata"])],
        description="Declared side effects for this command.",
    )
    path: str = Field(default=".", description="Directory path to search from.")
    name: str | None = Field(default=None, description="Optional filename pattern for -name.")
    type: Literal["file", "dir", "symlink"] | None = Field(
        default=None, description="Optional node kind filter."
    )
    maxdepth: int | None = Field(
        default=None, gt=0, description="Maximum directory depth to search."
    )

    def to_shell(self) -> str:
        tokens = [self._command_alias, self.path]
        if self.name:
            tokens.extend(["-name", self.name])
        if self.type:
            type_alias = {"file": "f", "dir": "d", "symlink": "l"}[self.type]
            tokens.extend(["-type", type_alias])
        if self.maxdepth is not None:
            tokens.extend(["-maxdepth", str(self.maxdepth)])
        return " ".join(tokens)
