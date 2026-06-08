from __future__ import annotations as _annotations

from vsh.schemas import RemoveCommand, StructuredCommand
from vsh.session import get_protected_path_label, is_same_path_or_ancestor, is_within_workspace
from vsh.snapshot.models import WorkspaceSnapshot

from .models import Overlay, PolicyDecision

_DANGEROUS_REMOVE_LITERALS = frozenset({".", "./", "..", "../", "~", "~/"})


def decide_policy(
    command: StructuredCommand,
    snapshot: WorkspaceSnapshot,
    overlay: Overlay,
) -> tuple[PolicyDecision, str | None]:
    dangerous_literal_reason = _dangerous_remove_literal_reason(command)
    if dangerous_literal_reason is not None:
        return "reject", dangerous_literal_reason

    destructive_boundary_reason = _destructive_boundary_reason(snapshot, overlay)
    if destructive_boundary_reason is not None:
        return "reject", destructive_boundary_reason

    protected_target_reason = _protected_target_reason(overlay)
    if protected_target_reason is not None:
        return "reject", protected_target_reason

    outside_workspace_reason = _outside_workspace_reason(snapshot, overlay)
    if outside_workspace_reason is not None:
        return "reject", outside_workspace_reason

    return "approve_with_warning", None


def _dangerous_remove_literal_reason(command: StructuredCommand) -> str | None:
    if not isinstance(command, RemoveCommand):
        return None
    normalized = command.path.strip()
    if normalized in {"~", "~/"}:
        return "destructive command cannot target the home directory shorthand"
    if normalized in _DANGEROUS_REMOVE_LITERALS:
        return f"destructive command cannot target shorthand path {normalized!r}"
    return None


def _destructive_boundary_reason(snapshot: WorkspaceSnapshot, overlay: Overlay) -> str | None:
    workspace_root = snapshot.session.workspace_root
    destructive_targets = set(overlay.deleted) | {src for src, _ in overlay.renames}
    for target in sorted(destructive_targets):
        if is_same_path_or_ancestor(target, workspace_root):
            return "destructive command cannot target the workspace root or one of its ancestors"
    return None


def _protected_target_reason(overlay: Overlay) -> str | None:
    destructive_targets = set(overlay.deleted) | {src for src, _ in overlay.renames}
    for target in sorted(destructive_targets):
        protected_label = get_protected_path_label(target)
        if protected_label is not None:
            return f"destructive command cannot target the {protected_label}"
    return None


def _outside_workspace_reason(snapshot: WorkspaceSnapshot, overlay: Overlay) -> str | None:
    workspace_root = snapshot.session.workspace_root
    mutation_targets = _mutation_targets(overlay)
    for target in sorted(mutation_targets):
        if not is_within_workspace(target, workspace_root):
            return f"mutating command target escapes workspace root: {target}"
    return None


def _mutation_targets(overlay: Overlay) -> set[str]:
    targets = set(overlay.created)
    targets.update(overlay.updated)
    targets.update(overlay.deleted)
    for src, dst in overlay.renames:
        targets.add(src)
        targets.add(dst)
    return targets
