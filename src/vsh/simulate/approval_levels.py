from __future__ import annotations as _annotations

import os
from typing import Literal

from vsh.schemas import (
    ApplyPatchCommand,
    ChmodCommand,
    CopyCommand,
    CurlCommand,
    EchoCommand,
    LnCommand,
    MkdirCommand,
    MoveCommand,
    RemoveCommand,
    SedCommand,
    StructuredCommand,
    TouchCommand,
    WgetCommand,
)
from vsh.simulate.models import Overlay, PolicyDecision

ApprovalTier = Literal["read_only", "mutation", "destructive"]

__all__ = (
    "ApprovalTier",
    "classify_approval_requirement",
    "max_touched_paths",
)


def max_touched_paths() -> int:
    raw = os.environ.get("VSH_MAX_TOUCHED_PATHS", "500")
    try:
        return max(1, int(raw))
    except ValueError:
        return 500


def classify_approval_requirement(
    command: StructuredCommand,
    *,
    decision: PolicyDecision,
    overlay: Overlay | None = None,
) -> tuple[ApprovalTier, bool]:
    """Return approval tier and whether explicit manual approval is required."""
    if decision == "reject":
        return "read_only", True

    if isinstance(command, WgetCommand):
        return "mutation", True
    if isinstance(command, CurlCommand):
        if command.output_path is not None:
            return "mutation", True
        return "read_only", True

    if isinstance(command, RemoveCommand):
        return "destructive", True

    if overlay is not None and overlay.deleted:
        return "destructive", True

    if _is_mutation_command(command, overlay):
        return "mutation", True

    if decision == "approve_with_warning":
        return "mutation", True

    return "read_only", False


def _is_mutation_command(command: StructuredCommand, overlay: Overlay | None) -> bool:
    if isinstance(
        command,
        MkdirCommand
        | TouchCommand
        | MoveCommand
        | CopyCommand
        | ChmodCommand
        | LnCommand
        | ApplyPatchCommand,
    ):
        return True
    if isinstance(command, EchoCommand) and command.output_path:
        return True
    if isinstance(command, SedCommand) and command.in_place:
        return True
    return overlay is not None and bool(
        overlay.created or overlay.updated or overlay.deleted or overlay.renames
    )
