from __future__ import annotations as _annotations

from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand


class ApplyPatchCommand(StructuredCommand):
    """Apply a unified diff or search-replace patch to a workspace file."""

    _command_alias: ClassVar[str] = "apply_patch"

    kind: CommandKind = Field(default="mutate", description="Patch application mutates files.")
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [SideEffect(kind="mutate", risks=["modifies file contents"])],
        description="Declared side effects for patch application.",
    )
    path: str = Field(description="Target file path relative to workspace cwd.")
    patch: str = Field(description="Unified diff text or search-replace patch body.")
    fuzzy: bool = Field(
        default=False,
        description="Allow fuzzy line matching when applying search-replace hunks.",
    )
