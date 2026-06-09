from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class GitStatusCommand(StructuredCommand):
    _command_alias: ClassVar[str] = "git_status"

    kind: CommandKind = Field(default="read")
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="read", risks=["reads git metadata"])],
    )
    path: str = Field(default=".", description="Repository path within workspace.")


class GitDiffCommand(StructuredCommand):
    _command_alias: ClassVar[str] = "git_diff"

    kind: CommandKind = Field(default="read")
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [
            SideEffect(kind="read", risks=["reads git metadata and file contents"])
        ],
    )
    path: str = Field(default=".", description="Repository path within workspace.")
    staged: bool = Field(default=False, description="Show staged diff only.")
