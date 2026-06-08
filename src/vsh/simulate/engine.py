from __future__ import annotations as _annotations

import uuid

from vsh.perf.timing import elapsed_ms, perf_counter_ns
from vsh.plans.fingerprint import compute_plan_fingerprint
from vsh.plans.models import SimulationResult
from vsh.runtime import runtime
from vsh.schemas import (
    CatCommand,
    CdCommand,
    ChmodCommand,
    CopyCommand,
    CurlCommand,
    DuCommand,
    EchoCommand,
    FindCommand,
    GrepCommand,
    HeadCommand,
    LnCommand,
    LsCommand,
    MkdirCommand,
    MoveCommand,
    NlCommand,
    PwdCommand,
    RemoveCommand,
    RgCommand,
    SedCommand,
    SortCommand,
    StatCommand,
    StructuredCommand,
    TailCommand,
    TouchCommand,
    WcCommand,
    WgetCommand,
)
from vsh.session import is_within_workspace, resolve_workspace_path
from vsh.snapshot.fingerprint import collect_touched_paths, fingerprints_for_paths
from vsh.snapshot.models import WorkspaceSnapshot

from .approval_levels import classify_approval_requirement, max_touched_paths
from .http_commands import simulate_curl_command, simulate_wget_command
from .models import AccessJournal, Overlay, PolicyDecision, PredictedEffects
from .policy import decide_policy, protected_read_path_reason

_READ_SCOPE_TREE_MIN_NODES = 128


def simulate_command(command: StructuredCommand, snapshot: WorkspaceSnapshot) -> SimulationResult:
    start_ns = perf_counter_ns()
    plan_id = f"plan_{uuid.uuid4().hex[:12]}"
    shell_preview = command.to_shell()
    overlay: Overlay | None = None
    if isinstance(command, PwdCommand):
        predicted = PredictedEffects(
            reads=[snapshot.session.cwd_logical], cwd_after=snapshot.session.cwd_logical
        )
        journal = AccessJournal(metadata_reads={snapshot.session.cwd_logical})
        decision: PolicyDecision = "approve"
        reason = None
    elif isinstance(command, CdCommand):
        target = resolve_workspace_path(snapshot.session.cwd_logical, command.path)
        journal = AccessJournal(metadata_reads={target}, cwd_changes=[target])
        if not is_within_workspace(target, snapshot.session.workspace_root):
            predicted = PredictedEffects(reads=[target], cwd_after=snapshot.session.cwd_logical)
            decision = "reject"
            reason = "target path escapes workspace root"
        else:
            predicted = PredictedEffects(reads=[target], cwd_after=target)
            decision = "approve"
            reason = None
    elif isinstance(command, LsCommand):
        target = resolve_workspace_path(snapshot.session.cwd_logical, command.path)
        node = snapshot.nodes.get(target)
        reads = [target]
        if node is not None:
            reads.extend(node.children)
        predicted, journal, decision, reason = _evaluate_read_target(
            snapshot,
            target,
            reads=reads,
        )
    elif isinstance(
        command, CatCommand | HeadCommand | NlCommand | SortCommand | TailCommand | WcCommand
    ):
        target = resolve_workspace_path(snapshot.session.cwd_logical, command.path)
        predicted, journal, decision, reason = _evaluate_read_target(
            snapshot,
            target,
            include_content_reads=True,
        )
    elif isinstance(command, SedCommand) and not command.in_place:
        targets = [
            resolve_workspace_path(snapshot.session.cwd_logical, path) for path in command.paths
        ]
        predicted, journal, decision, reason = _evaluate_read_targets(
            snapshot,
            targets,
            include_content_reads=True,
        )
    elif isinstance(command, DuCommand | StatCommand):
        target = resolve_workspace_path(snapshot.session.cwd_logical, command.path)
        reads = _read_scope(snapshot, target)
        predicted, journal, decision, reason = _evaluate_read_target(snapshot, target, reads=reads)
    elif isinstance(command, GrepCommand | RgCommand):
        target = resolve_workspace_path(snapshot.session.cwd_logical, command.path)
        reads = _read_scope(snapshot, target)
        predicted, journal, decision, reason = _evaluate_read_target(
            snapshot,
            target,
            reads=reads,
            include_content_reads=True,
        )
    elif isinstance(command, FindCommand):
        target = resolve_workspace_path(snapshot.session.cwd_logical, command.path)
        reads = _read_scope(snapshot, target)
        predicted, journal, decision, reason = _evaluate_read_target(snapshot, target, reads=reads)
    elif isinstance(command, EchoCommand) and command.output_path is None:
        predicted = PredictedEffects(cwd_after=snapshot.session.cwd_logical)
        journal = AccessJournal()
        decision = "approve"
        reason = None
    elif isinstance(command, CurlCommand):
        predicted, journal, decision, reason, overlay = simulate_curl_command(command, snapshot)
    elif isinstance(command, WgetCommand):
        predicted, journal, decision, reason, overlay = simulate_wget_command(command, snapshot)
    else:
        overlay = _simulate_mutation_overlay(command, snapshot)
        journal = AccessJournal(
            metadata_reads={snapshot.session.cwd_logical},
            metadata_writes=set(overlay.created) | set(overlay.updated),
            content_writes=set(overlay.updated),
            creates=set(overlay.created),
            deletes=set(overlay.deleted),
            renames=list(overlay.renames),
            cwd_changes=[overlay.cwd_override] if overlay.cwd_override else [],
        )
        predicted = PredictedEffects(
            reads=[snapshot.session.cwd_logical],
            creates=list(overlay.created),
            deletes=list(overlay.deleted),
            updates=list(overlay.updated),
            renames=list(overlay.renames),
            cwd_after=overlay.cwd_override or snapshot.session.cwd_logical,
        )
        decision, reason = decide_policy(command, snapshot, overlay)
    touched_paths = collect_touched_paths(journal, predicted)
    if decision != "reject":
        limit = max_touched_paths()
        if len(touched_paths) > limit:
            decision = "reject"
            reason = f"command would touch too many paths ({len(touched_paths)} > {limit})"
    raw_matches_shell_preview = command.raw_matches_shell_preview(shell_preview)
    execution_eligible, execution_eligibility_reason = _evaluate_execution_eligibility(
        decision=decision,
        raw_matches_shell_preview=raw_matches_shell_preview,
    )
    approval_tier, requires_manual_approval = classify_approval_requirement(
        command,
        decision=decision,
        overlay=overlay,
    )
    result = SimulationResult(
        plan_id=plan_id,
        command=command,
        shell_preview=shell_preview,
        decision=decision,
        reason=reason,
        raw_matches_shell_preview=raw_matches_shell_preview,
        execution_eligible=execution_eligible,
        execution_eligibility_reason=execution_eligibility_reason,
        approval_tier=approval_tier,
        requires_manual_approval=requires_manual_approval,
        predicted_effects=predicted,
        journal=journal,
        simulation_time_ms=elapsed_ms(start_ns),
    )
    path_fingerprints = fingerprints_for_paths(touched_paths, snapshot)
    plan_fingerprint = compute_plan_fingerprint(
        snapshot_id=snapshot.snapshot_id,
        command=command,
        shell_preview=shell_preview,
        path_fingerprints=path_fingerprints,
    )
    runtime.record_plan(
        result,
        snapshot.snapshot_id,
        path_fingerprints=path_fingerprints,
        plan_fingerprint=plan_fingerprint,
    )
    return result


def _simulate_mutation_overlay(command: StructuredCommand, snapshot: WorkspaceSnapshot) -> Overlay:
    cwd = snapshot.session.cwd_logical
    overlay = Overlay()
    if isinstance(command, MkdirCommand):
        target = resolve_workspace_path(cwd, command.path)
        overlay.created[target] = {"kind": "dir", "parents": command.parents}
    elif isinstance(command, TouchCommand):
        target = resolve_workspace_path(cwd, command.path)
        overlay.updated[target] = {"kind": "file", "no_create": command.no_create}
        if not command.no_create:
            overlay.created.setdefault(target, {"kind": "file"})
    elif isinstance(command, MoveCommand):
        src = resolve_workspace_path(cwd, command.src)
        dst = resolve_workspace_path(cwd, command.dst)
        overlay.renames.append((src, dst))
        overlay.updated[dst] = {"source": src, "overwrite": command.overwrite}
    elif isinstance(command, CopyCommand):
        src = resolve_workspace_path(cwd, command.src)
        dst = resolve_workspace_path(cwd, command.dst)
        overlay.created[dst] = {
            "source": src,
            "recursive": command.recursive,
            "overwrite": command.overwrite,
        }
    elif isinstance(command, RemoveCommand):
        target = resolve_workspace_path(cwd, command.path)
        overlay.deleted.add(target)
    elif isinstance(command, EchoCommand):
        if command.output_path:
            target = resolve_workspace_path(cwd, command.output_path)
            overlay.updated[target] = {"kind": "file", "append": command.append}
            if not command.append:
                overlay.created.setdefault(target, {"kind": "file"})
    elif isinstance(command, ChmodCommand):
        target = resolve_workspace_path(cwd, command.path)
        overlay.updated[target] = {"mode": command.mode, "recursive": command.recursive}
    elif isinstance(command, LnCommand):
        src = resolve_workspace_path(cwd, command.src)
        dst = resolve_workspace_path(cwd, command.dst)
        overlay.created[dst] = {"source": src, "symbolic": command.symbolic, "force": command.force}
    elif isinstance(command, SedCommand):
        for path in command.paths:
            target = resolve_workspace_path(cwd, path)
            overlay.updated[target] = {"script": command.script, "in_place": command.in_place}
    else:
        overlay.updated[cwd] = {"command": command.__class__.__name__}
    return overlay


def _read_scope(snapshot: WorkspaceSnapshot, target: str) -> list[str]:
    node = snapshot.nodes.get(target)
    if node is None:
        return [target]
    if node.kind != "dir":
        return [target]
    if (
        target == snapshot.session.workspace_root
        or len(snapshot.nodes) <= _READ_SCOPE_TREE_MIN_NODES
    ):
        prefix = f"{target.rstrip('/')}/"
        return [path for path in snapshot.nodes if path == target or path.startswith(prefix)]
    scoped: list[str] = []
    stack = [target]
    while stack:
        path = stack.pop()
        scoped.append(path)
        current = snapshot.nodes.get(path)
        if current is not None and current.kind == "dir":
            stack.extend(reversed(current.children))
    return scoped


def _first_outside_workspace(targets: list[str], snapshot: WorkspaceSnapshot) -> str | None:
    for target in targets:
        if not is_within_workspace(target, snapshot.session.workspace_root):
            return target
    return None


def _first_protected_target(targets: list[str], workspace_root: str) -> str | None:
    for target in targets:
        if protected_read_path_reason(target, workspace_root) is not None:
            return target
    return None


def _evaluate_read_target(
    snapshot: WorkspaceSnapshot,
    target: str,
    *,
    reads: list[str] | None = None,
    include_content_reads: bool = False,
) -> tuple[PredictedEffects, AccessJournal, PolicyDecision, str | None]:
    return _evaluate_read_targets(
        snapshot,
        reads if reads is not None else [target],
        include_content_reads=include_content_reads,
    )


def _evaluate_read_targets(
    snapshot: WorkspaceSnapshot,
    targets: list[str],
    *,
    include_content_reads: bool = False,
) -> tuple[PredictedEffects, AccessJournal, PolicyDecision, str | None]:
    outside_target = _first_outside_workspace(targets, snapshot)
    if outside_target is not None:
        return (
            PredictedEffects(reads=[outside_target], cwd_after=snapshot.session.cwd_logical),
            AccessJournal(metadata_reads={outside_target}),
            "reject",
            "target path escapes workspace root",
        )
    protected_target = _first_protected_target(targets, snapshot.session.workspace_root)
    if protected_target is not None:
        reason = protected_read_path_reason(protected_target, snapshot.session.workspace_root)
        return (
            PredictedEffects(reads=[protected_target], cwd_after=snapshot.session.cwd_logical),
            AccessJournal(metadata_reads={protected_target}),
            "reject",
            reason,
        )
    journal = AccessJournal(metadata_reads=set(targets))
    if include_content_reads:
        journal = AccessJournal(metadata_reads=set(targets), content_reads=set(targets))
    return (
        PredictedEffects(reads=targets, cwd_after=snapshot.session.cwd_logical),
        journal,
        "approve",
        None,
    )


def _evaluate_execution_eligibility(
    *,
    decision: PolicyDecision,
    raw_matches_shell_preview: bool | None,
) -> tuple[bool, str | None]:
    if decision == "reject":
        return False, f"simulation decision {decision!r} is not execution-eligible"
    if raw_matches_shell_preview is False:
        return False, "raw command does not match the canonical shell preview"
    return True, None
