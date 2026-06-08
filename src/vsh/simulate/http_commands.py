from __future__ import annotations as _annotations

from vsh.http import default_wget_output_name, validate_http_url
from vsh.schemas import CurlCommand, WgetCommand
from vsh.session import is_within_workspace, resolve_workspace_path
from vsh.simulate.models import AccessJournal, Overlay, PolicyDecision, PredictedEffects
from vsh.simulate.policy import decide_policy
from vsh.simulate.protected_paths import get_protected_workspace_path_reason
from vsh.snapshot.models import WorkspaceSnapshot

__all__ = ("simulate_curl_command", "simulate_wget_command")


def simulate_curl_command(
    command: CurlCommand,
    snapshot: WorkspaceSnapshot,
) -> tuple[PredictedEffects, AccessJournal, PolicyDecision, str | None, Overlay | None]:
    cwd = snapshot.session.cwd_logical
    workspace_root = snapshot.session.workspace_root
    try:
        validated_url = validate_http_url(command.url)
    except ValueError as exc:
        return (
            PredictedEffects(cwd_after=cwd),
            AccessJournal(),
            "reject",
            str(exc),
            None,
        )

    if command.output_path is None:
        return (
            PredictedEffects(reads=[validated_url], cwd_after=cwd),
            AccessJournal(metadata_reads={validated_url}),
            "approve",
            None,
            None,
        )

    target = resolve_workspace_path(cwd, command.output_path)
    if not is_within_workspace(target, workspace_root):
        return (
            PredictedEffects(reads=[validated_url], cwd_after=cwd),
            AccessJournal(metadata_reads={validated_url}),
            "reject",
            f"output path escapes workspace root: {target}",
            None,
        )
    protected_reason = get_protected_workspace_path_reason(target, workspace_root)
    if protected_reason is not None:
        return (
            PredictedEffects(reads=[validated_url], cwd_after=cwd),
            AccessJournal(metadata_reads={validated_url}),
            "reject",
            protected_reason,
            None,
        )

    overlay = Overlay()
    if target in snapshot.nodes:
        overlay.updated[target] = {"kind": "file", "source": "curl"}
    else:
        overlay.created[target] = {"kind": "file", "source": "curl"}

    journal = AccessJournal(
        metadata_reads={validated_url, cwd},
        metadata_writes={target},
        content_writes={target},
        creates=set(overlay.created),
    )
    predicted = PredictedEffects(
        reads=[validated_url, cwd],
        creates=list(overlay.created),
        updates=list(overlay.updated),
        cwd_after=cwd,
    )
    decision, reason = decide_policy(command, snapshot, overlay)
    return predicted, journal, decision, reason, overlay


def simulate_wget_command(
    command: WgetCommand,
    snapshot: WorkspaceSnapshot,
) -> tuple[PredictedEffects, AccessJournal, PolicyDecision, str | None, Overlay | None]:
    cwd = snapshot.session.cwd_logical
    workspace_root = snapshot.session.workspace_root
    try:
        validated_url = validate_http_url(command.url)
    except ValueError as exc:
        return (
            PredictedEffects(cwd_after=cwd),
            AccessJournal(),
            "reject",
            str(exc),
            None,
        )

    relative_output = command.output_path or default_wget_output_name(command.url)
    target = resolve_workspace_path(cwd, relative_output)
    if not is_within_workspace(target, workspace_root):
        return (
            PredictedEffects(reads=[validated_url], cwd_after=cwd),
            AccessJournal(metadata_reads={validated_url}),
            "reject",
            f"output path escapes workspace root: {target}",
            None,
        )
    protected_reason = get_protected_workspace_path_reason(target, workspace_root)
    if protected_reason is not None:
        return (
            PredictedEffects(reads=[validated_url], cwd_after=cwd),
            AccessJournal(metadata_reads={validated_url}),
            "reject",
            protected_reason,
            None,
        )

    overlay = Overlay()
    if target in snapshot.nodes:
        overlay.updated[target] = {"kind": "file", "source": "wget"}
    else:
        overlay.created[target] = {"kind": "file", "source": "wget"}

    journal = AccessJournal(
        metadata_reads={validated_url, cwd},
        metadata_writes={target},
        content_writes={target},
        creates=set(overlay.created),
    )
    predicted = PredictedEffects(
        reads=[validated_url, cwd],
        creates=list(overlay.created),
        updates=list(overlay.updated),
        cwd_after=cwd,
    )
    decision, reason = decide_policy(command, snapshot, overlay)
    return predicted, journal, decision, reason, overlay
