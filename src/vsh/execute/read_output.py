from __future__ import annotations as _annotations

import fnmatch
import os
import re
import stat
from pathlib import Path

from vsh.limits import (
    compile_grep_pattern,
    grep_max_file_bytes,
    grep_max_matches,
    read_max_file_bytes,
)
from vsh.schemas import (
    CatCommand,
    DuCommand,
    FindCommand,
    GrepCommand,
    HeadCommand,
    LsCommand,
    NlCommand,
    RgCommand,
    SedCommand,
    SortCommand,
    StatCommand,
    StructuredCommand,
    TailCommand,
    WcCommand,
)
from vsh.snapshot.constants import IGNORED_DIRECTORIES

from .dispatch import ExecutionContext

__all__ = ("capture_read_output",)


def capture_read_output(command: StructuredCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    """Resolve read paths and capture stdout for read-only commands."""
    if isinstance(command, LsCommand):
        return _ls_output(command, ctx)
    if isinstance(command, CatCommand):
        return _cat_output(command, ctx)
    if isinstance(command, HeadCommand):
        return _head_output(command, ctx)
    if isinstance(command, TailCommand):
        return _tail_output(command, ctx)
    if isinstance(command, NlCommand):
        return _nl_output(command, ctx)
    if isinstance(command, SortCommand):
        return _sort_output(command, ctx)
    if isinstance(command, WcCommand):
        return _wc_output(command, ctx)
    if isinstance(command, StatCommand):
        return _stat_output(command, ctx)
    if isinstance(command, DuCommand):
        return _du_output(command, ctx)
    if isinstance(command, GrepCommand):
        return _grep_output(command, ctx)
    if isinstance(command, RgCommand):
        return _rg_output(command, ctx)
    if isinstance(command, FindCommand):
        return _find_output(command, ctx)
    if isinstance(command, SedCommand) and not command.in_place:
        return _sed_read_output(command, ctx)
    msg = f"unsupported read command for output capture: {command.__class__.__name__}"
    raise ValueError(msg)


def _ls_output(command: LsCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    target = ctx.resolve_within_workspace(command.path)
    path = Path(target)
    _require_exists(target)
    if not path.is_dir():
        msg = f"ls target is not a directory: {target}"
        raise ValueError(msg)
    lines = _render_ls_directory(path, command)
    stdout = "\n".join(lines)
    if stdout:
        stdout += "\n"
    return [target], stdout


def _render_ls_directory(path: Path, command: LsCommand, *, prefix: str = "") -> list[str]:
    entries = sorted(path.iterdir(), key=lambda item: item.name)
    if not command.all:
        entries = [entry for entry in entries if not entry.name.startswith(".")]
    lines: list[str] = []
    for entry in entries:
        display_name = f"{prefix}{entry.name}" if prefix else entry.name
        if command.long:
            mode = stat.filemode(entry.stat().st_mode)
            lines.append(f"{mode} {display_name}")
        else:
            lines.append(display_name)
        if command.recursive and entry.is_dir():
            lines.extend(
                _render_ls_directory(
                    entry,
                    command,
                    prefix=f"{display_name}/" if prefix else f"{entry.name}/",
                )
            )
    return lines


def _cat_output(command: CatCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    target = ctx.resolve_within_workspace(command.path)
    _require_exists(target)
    raw = Path(target).read_bytes()
    if len(raw) > read_max_file_bytes():
        msg = f"file exceeds read max bytes ({read_max_file_bytes()})"
        raise ValueError(msg)
    content = raw.decode("utf-8")
    lines = content.splitlines()
    if command.squeeze_blank:
        lines = [line for index, line in enumerate(lines) if line or index == 0 or lines[index - 1]]
    rendered: list[str] = []
    for index, line in enumerate(lines, start=1):
        body = line.replace("\t", "^I") if command.show_ends else line
        if command.show_ends:
            body = f"{body}$"
        if command.number:
            rendered.append(f"{index:6}\t{body}")
        else:
            rendered.append(body)
    stdout = "\n".join(rendered)
    if content.endswith("\n") and stdout:
        stdout += "\n"
    return [target], stdout


def _head_output(command: HeadCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    target = ctx.resolve_within_workspace(command.path)
    _require_exists(target)
    lines = Path(target).read_text(encoding="utf-8").splitlines()
    stdout = "\n".join(lines[: command.lines])
    if stdout:
        stdout += "\n"
    return [target], stdout


def _tail_output(command: TailCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    target = ctx.resolve_within_workspace(command.path)
    _require_exists(target)
    if command.follow:
        msg = "tail --follow is not supported for execution output capture"
        raise ValueError(msg)
    lines = Path(target).read_text(encoding="utf-8").splitlines()
    stdout = "\n".join(lines[-command.lines :])
    if stdout:
        stdout += "\n"
    return [target], stdout


def _nl_output(command: NlCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    target = ctx.resolve_within_workspace(command.path)
    _require_exists(target)
    lines = Path(target).read_text(encoding="utf-8").splitlines()
    rendered: list[str] = []
    for index, line in enumerate(lines, start=1):
        if command.number_all or line:
            rendered.append(f"{index:6}\t{line}")
        else:
            rendered.append(line)
    stdout = "\n".join(rendered)
    if stdout:
        stdout += "\n"
    return [target], stdout


def _sort_output(command: SortCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    target = ctx.resolve_within_workspace(command.path)
    _require_exists(target)
    lines = Path(target).read_text(encoding="utf-8").splitlines()
    lines.sort(reverse=command.reverse)
    if command.unique:
        unique_lines: list[str] = []
        for line in lines:
            if not unique_lines or line != unique_lines[-1]:
                unique_lines.append(line)
        lines = unique_lines
    stdout = "\n".join(lines)
    if stdout:
        stdout += "\n"
    return [target], stdout


def _wc_output(command: WcCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    target = ctx.resolve_within_workspace(command.path)
    _require_exists(target)
    content = Path(target).read_text(encoding="utf-8")
    counts: list[str] = []
    if command.lines:
        counts.append(
            str(content.count("\n") + (0 if content.endswith("\n") or not content else 1))
        )
    if command.words:
        counts.append(str(len(content.split())))
    if command.bytes:
        counts.append(str(len(content.encode("utf-8"))))
    if command.chars:
        counts.append(str(len(content)))
    if not counts:
        line_count = content.count("\n") + (0 if content.endswith("\n") or not content else 1)
        counts = [str(line_count), str(len(content.split())), str(len(content.encode("utf-8")))]
    return [target], f"{' '.join(counts)} {target}\n"


def _stat_output(command: StatCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    target = ctx.resolve_within_workspace(command.path)
    _require_exists(target)
    info = Path(target).stat()
    stdout = f"  File: {target}\n  Size: {info.st_size}\n  Mode: {stat.filemode(info.st_mode)}\n"
    return [target], stdout


def _du_output(command: DuCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    target = ctx.resolve_within_workspace(command.path)
    _require_exists(target)
    total = _directory_size(Path(target))
    rendered = _human_size(total) if command.human_readable else str(total)
    stdout = f"{rendered}\t{target}\n"
    return [target], stdout


def _grep_output(command: GrepCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    root = ctx.resolve_within_workspace(command.path)
    _require_exists(root)
    matches, reads = _search_paths(
        Path(root),
        pattern=command.pattern,
        ignore_case=command.ignore_case,
        line_number=command.line_number,
        fixed_strings=command.fixed_strings,
        extended_regexp=command.extended_regexp,
        recursive=command.recursive,
    )
    return reads, "".join(matches)


def _rg_output(command: RgCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    root = ctx.resolve_within_workspace(command.path)
    _require_exists(root)
    matches, reads = _search_paths(
        Path(root),
        pattern=command.pattern,
        ignore_case=command.ignore_case,
        line_number=command.line_number,
        fixed_strings=command.fixed_strings,
        recursive=True,
        include_hidden=command.hidden,
    )
    return reads, "".join(matches)


def _find_output(command: FindCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    root = ctx.resolve_within_workspace(command.path)
    _require_exists(root)
    matches: list[str] = []
    reads: list[str] = [root]
    for current_root, dirs, files in os.walk(root):
        depth = current_root[len(root) :].count(os.sep)
        if command.maxdepth is not None and depth >= command.maxdepth:
            dirs[:] = []
        for name in sorted(dirs + files):
            candidate = Path(current_root) / name
            resolved = str(candidate.resolve())
            reads.append(resolved)
            if command.name and not fnmatch.fnmatch(name, command.name):
                continue
            if command.type == "file" and not candidate.is_file():
                continue
            if command.type == "dir" and not candidate.is_dir():
                continue
            if command.type == "symlink" and not candidate.is_symlink():
                continue
            matches.append(resolved)
    stdout = "\n".join(matches)
    if stdout:
        stdout += "\n"
    return sorted(set(reads)), stdout


def _sed_read_output(command: SedCommand, ctx: ExecutionContext) -> tuple[list[str], str]:
    reads = [ctx.resolve_within_workspace(path) for path in command.paths]
    for target in reads:
        _require_exists(target)
    chunks: list[str] = []
    for target in reads:
        content = Path(target).read_text(encoding="utf-8")
        chunks.append(_apply_sed_read_script(content, command.script))
    stdout = "\n".join(chunk for chunk in chunks if chunk)
    if stdout and not stdout.endswith("\n"):
        stdout += "\n"
    return reads, stdout


def _apply_sed_read_script(content: str, script: str) -> str:
    print_match = re.fullmatch(r"(\d+)p", script)
    if print_match is not None:
        lines = content.splitlines()
        line_number = int(print_match.group(1))
        if 1 <= line_number <= len(lines):
            return f"{lines[line_number - 1]}\n"
        return ""
    substitute_match = re.fullmatch(r"s/([^/]+)/([^/]*)/g", script)
    if substitute_match is not None:
        return content.replace(substitute_match.group(1), substitute_match.group(2))
    msg = f"unsupported sed script for execution: {script!r}"
    raise ValueError(msg)


def _search_paths(
    root: Path,
    *,
    pattern: str,
    ignore_case: bool,
    line_number: bool,
    fixed_strings: bool,
    extended_regexp: bool = False,
    recursive: bool,
    include_hidden: bool = False,
) -> tuple[list[str], list[str]]:
    files = _iter_files(root, recursive=recursive, include_hidden=include_hidden)
    matches: list[str] = []
    reads: list[str] = []
    compiled = compile_grep_pattern(
        pattern,
        ignore_case=ignore_case,
        fixed_strings=fixed_strings,
        extended_regexp=extended_regexp,
    )
    max_matches = grep_max_matches()
    max_file_bytes = grep_max_file_bytes()
    for file_path in files:
        resolved = str(file_path.resolve())
        reads.append(resolved)
        try:
            if file_path.stat().st_size > max_file_bytes:
                continue
            lines = file_path.read_text(encoding="utf-8").splitlines()
        except (OSError, UnicodeDecodeError):
            continue
        for line_number_index, line in enumerate(lines, start=1):
            if fixed_strings:
                matched = pattern in line
            else:
                assert compiled is not None
                matched = compiled.search(line) is not None
            if not matched:
                continue
            prefix = f"{file_path}:{line_number_index}:" if line_number else f"{file_path}:"
            matches.append(f"{prefix}{line}\n")
            if len(matches) >= max_matches:
                return matches, sorted(set(reads))
    return matches, sorted(set(reads))


def _iter_files(root: Path, *, recursive: bool, include_hidden: bool) -> list[Path]:
    if root.is_file():
        return [root]
    files: list[Path] = []
    if recursive:
        for current_root, dirs, filenames in os.walk(root):
            dirs[:] = [
                name
                for name in dirs
                if name not in IGNORED_DIRECTORIES and (include_hidden or not name.startswith("."))
            ]
            if not include_hidden:
                filenames = [name for name in filenames if not name.startswith(".")]
            for name in filenames:
                files.append(Path(current_root) / name)
    else:
        for entry in root.iterdir():
            if not include_hidden and entry.name.startswith("."):
                continue
            if entry.is_file():
                files.append(entry)
    return files


def _directory_size(path: Path) -> int:
    if path.is_file():
        return path.stat().st_size
    total = 0
    for child in path.rglob("*"):
        if child.is_file():
            total += child.stat().st_size
    return total


def _human_size(size: int) -> str:
    units = ["B", "K", "M", "G", "T"]
    value = float(size)
    unit_index = 0
    while unit_index < len(units) - 1 and value >= 1024:
        value /= 1024
        unit_index += 1
    unit = units[unit_index]
    if unit == "B":
        return f"{int(value)}{unit}"
    return f"{value:.1f}{unit}"


def _require_exists(path: str) -> None:
    if not Path(path).exists():
        msg = f"path does not exist: {path}"
        raise FileNotFoundError(msg)
