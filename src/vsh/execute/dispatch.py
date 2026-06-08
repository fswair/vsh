from __future__ import annotations as _annotations

import os
import re
import shutil
import stat
from pathlib import Path

from vsh.effects import ActualEffects
from vsh.perf.timing import perf_counter_ns, stamp_execution_time
from vsh.schemas import (
    CatCommand,
    CdCommand,
    ChmodCommand,
    CopyCommand,
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
)
from vsh.session import is_within_workspace, resolve_workspace_path

__all__ = ("ExecutionContext", "apply_command", "effects_match_prediction")


class ExecutionContext:
    __slots__ = ("cwd_logical", "workspace_root")

    def __init__(self, *, workspace_root: str, cwd_logical: str) -> None:
        self.workspace_root = workspace_root
        self.cwd_logical = cwd_logical

    def resolve(self, candidate: str) -> str:
        return resolve_workspace_path(self.cwd_logical, candidate)

    def resolve_within_workspace(self, candidate: str) -> str:
        target = self.resolve(candidate)
        if not is_within_workspace(target, self.workspace_root):
            msg = f"path escapes workspace root: {target}"
            raise ValueError(msg)
        return target


def apply_command(command: StructuredCommand, ctx: ExecutionContext) -> ActualEffects:
    start_ns = perf_counter_ns()
    effects = _apply_command_body(command, ctx)
    stamped = stamp_execution_time(effects, start_ns)
    assert isinstance(stamped, ActualEffects)
    return stamped


def _apply_command_body(command: StructuredCommand, ctx: ExecutionContext) -> ActualEffects:
    from vsh.execute.read_output import capture_read_output

    if isinstance(command, PwdCommand):
        return ActualEffects(reads=[ctx.cwd_logical], cwd_after=ctx.cwd_logical)
    if isinstance(command, CdCommand):
        target = ctx.resolve_within_workspace(command.path)
        if not Path(target).is_dir():
            msg = f"cd target is not a directory: {target}"
            raise ValueError(msg)
        return ActualEffects(reads=[target], cwd_after=target)
    if isinstance(command, LsCommand):
        reads, stdout = capture_read_output(command, ctx)
        return ActualEffects(reads=reads, cwd_after=ctx.cwd_logical, stdout=stdout)
    if isinstance(command, MkdirCommand):
        target = ctx.resolve_within_workspace(command.path)
        if command.parents:
            Path(target).mkdir(parents=True, exist_ok=True)
        else:
            Path(target).mkdir()
        return ActualEffects(creates=[target], cwd_after=ctx.cwd_logical)
    if isinstance(command, TouchCommand):
        target = ctx.resolve_within_workspace(command.path)
        path = Path(target)
        if command.no_create and not path.exists():
            msg = f"touch target does not exist: {target}"
            raise ValueError(msg)
        path.parent.mkdir(parents=True, exist_ok=True)
        existed_before = path.exists()
        path.touch()
        if existed_before:
            return ActualEffects(updates=[target], cwd_after=ctx.cwd_logical)
        return ActualEffects(creates=[target], cwd_after=ctx.cwd_logical)
    if isinstance(command, MoveCommand):
        src = ctx.resolve_within_workspace(command.src)
        dst = ctx.resolve_within_workspace(command.dst)
        _require_exists(src)
        if Path(dst).exists() and not command.overwrite:
            msg = f"move destination already exists: {dst}"
            raise ValueError(msg)
        Path(dst).parent.mkdir(parents=True, exist_ok=True)
        shutil.move(src, dst)
        return ActualEffects(
            deletes=[src],
            creates=[dst],
            renames=[(src, dst)],
            cwd_after=ctx.cwd_logical,
        )
    if isinstance(command, CopyCommand):
        src = ctx.resolve_within_workspace(command.src)
        dst = ctx.resolve_within_workspace(command.dst)
        _require_exists(src)
        source_path = Path(src)
        destination_path = Path(dst)
        if destination_path.exists() and not command.overwrite:
            msg = f"copy destination already exists: {dst}"
            raise ValueError(msg)
        destination_path.parent.mkdir(parents=True, exist_ok=True)
        if source_path.is_dir():
            if command.recursive:
                shutil.copytree(src, dst, dirs_exist_ok=command.overwrite)
            else:
                msg = f"copy source is a directory but recursive is false: {src}"
                raise ValueError(msg)
        else:
            shutil.copy2(src, dst)
        return ActualEffects(creates=[dst], cwd_after=ctx.cwd_logical)
    if isinstance(command, RemoveCommand):
        target = ctx.resolve_within_workspace(command.path)
        path = Path(target)
        _require_exists(target)
        if path.is_dir():
            if command.recursive:
                shutil.rmtree(target)
            else:
                msg = f"remove target is a directory but recursive is false: {target}"
                raise ValueError(msg)
        else:
            path.unlink()
        return ActualEffects(deletes=[target], cwd_after=ctx.cwd_logical)
    if isinstance(command, EchoCommand):
        if command.output_path is None:
            return ActualEffects(cwd_after=ctx.cwd_logical)
        target = ctx.resolve_within_workspace(command.output_path)
        path = Path(target)
        path.parent.mkdir(parents=True, exist_ok=True)
        existed_before = path.exists()
        mode = "a" if command.append else "w"
        with path.open(mode, encoding="utf-8") as handle:
            handle.write(command.text)
            if not command.no_newline:
                handle.write("\n")
        if existed_before or command.append:
            return ActualEffects(updates=[target], cwd_after=ctx.cwd_logical)
        return ActualEffects(creates=[target], cwd_after=ctx.cwd_logical)
    if isinstance(command, ChmodCommand):
        target = ctx.resolve_within_workspace(command.path)
        path = Path(target)
        _require_exists(target)
        mode = _parse_mode(command.mode, path.stat().st_mode)
        if command.recursive and path.is_dir():
            for root, dirs, files in os.walk(target):
                os.chmod(root, mode)
                for name in files:
                    os.chmod(os.path.join(root, name), mode)
                for name in dirs:
                    os.chmod(os.path.join(root, name), mode)
        else:
            os.chmod(target, mode)
        return ActualEffects(updates=[target], cwd_after=ctx.cwd_logical)
    if isinstance(command, LnCommand):
        src = ctx.resolve_within_workspace(command.src)
        dst = ctx.resolve_within_workspace(command.dst)
        _require_exists(src)
        if Path(dst).exists() and not command.force:
            msg = f"link destination already exists: {dst}"
            raise ValueError(msg)
        if command.symbolic:
            os.symlink(src, dst)
        else:
            os.link(src, dst)
        return ActualEffects(creates=[dst], cwd_after=ctx.cwd_logical)
    if isinstance(command, SedCommand):
        if not command.in_place:
            reads, stdout = capture_read_output(command, ctx)
            return ActualEffects(reads=reads, cwd_after=ctx.cwd_logical, stdout=stdout)
        updated: list[str] = []
        for relative_path in command.paths:
            target = ctx.resolve_within_workspace(relative_path)
            _apply_sed_script(target, command.script, command.backup_suffix)
            updated.append(target)
        return ActualEffects(updates=updated, cwd_after=ctx.cwd_logical)
    if isinstance(
        command,
        CatCommand
        | HeadCommand
        | TailCommand
        | NlCommand
        | WcCommand
        | SortCommand
        | DuCommand
        | StatCommand
        | GrepCommand
        | RgCommand
        | FindCommand,
    ):
        reads, stdout = capture_read_output(command, ctx)
        return ActualEffects(reads=reads, cwd_after=ctx.cwd_logical, stdout=stdout)
    msg = f"unsupported command for real execution: {command.__class__.__name__}"
    raise ValueError(msg)


def effects_match_prediction(predicted: object, actual: ActualEffects) -> bool:
    from vsh.simulate.models import PredictedEffects

    if not isinstance(predicted, PredictedEffects):
        return False
    return (
        sorted(predicted.creates) == sorted(actual.creates)
        and sorted(predicted.updates) == sorted(actual.updates)
        and sorted(predicted.deletes) == sorted(actual.deletes)
        and sorted(predicted.renames) == sorted(actual.renames)
        and predicted.cwd_after == actual.cwd_after
    )


def _require_exists(path: str) -> None:
    if not Path(path).exists():
        msg = f"path does not exist: {path}"
        raise FileNotFoundError(msg)


def _parse_mode(mode: str, current_mode: int) -> int:
    if mode.isdigit():
        return int(mode, 8)
    if mode in {"+x", "u+x"}:
        return current_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH
    msg = f"unsupported chmod mode: {mode!r}"
    raise ValueError(msg)


def _apply_sed_script(path: str, script: str, backup_suffix: str | None) -> None:
    target = Path(path)
    content = target.read_text(encoding="utf-8")
    match = re.fullmatch(r"s/([^/]+)/([^/]*)/g", script)
    if match is None:
        msg = f"unsupported sed script for execution: {script!r}"
        raise ValueError(msg)
    updated = content.replace(match.group(1), match.group(2))
    if backup_suffix is not None:
        backup_path = target.with_name(target.name + backup_suffix)
        backup_path.write_text(content, encoding="utf-8")
    target.write_text(updated, encoding="utf-8")
