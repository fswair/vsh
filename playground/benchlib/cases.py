from __future__ import annotations as _annotations

import shutil
import sys
from pathlib import Path

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
    TailCommand,
    TouchCommand,
    WcCommand,
)

from .models import BenchmarkCase

SAMPLE = "file_0001.txt"
COPY_NAME = f"copy_{SAMPLE}"
MOVED_NAME = "moved_file_0001.txt"
LINK_NAME = "bench_link.txt"
ECHO_OUT = "bench_echo_out.txt"
TOUCH_PATH = "bench_touch.txt"
MKDIR_PATH = "bench_mkdir/nested"
SED_RESTORE = "needle special 1\n"


def _unlink_if_exists(root: Path, relative: str) -> None:
    target = root / relative
    if target.is_file():
        target.unlink()
    elif target.is_dir():
        shutil.rmtree(target)


def _restore_sample(root: Path) -> None:
    sample = root / SAMPLE
    payload = sample.read_text(encoding="utf-8")
    if "needle" not in payload:
        sample.write_text(f"{SED_RESTORE}needle line\n", encoding="utf-8")


def build_cases() -> list[BenchmarkCase]:
    def prepare_copy(root: Path) -> None:
        _unlink_if_exists(root, COPY_NAME)

    def prepare_remove(root: Path) -> None:
        shutil.copy2(root / SAMPLE, root / COPY_NAME)

    def prepare_touch(root: Path) -> None:
        _unlink_if_exists(root, TOUCH_PATH)

    def prepare_mkdir(root: Path) -> None:
        _unlink_if_exists(root, "bench_mkdir")

    def prepare_ln(root: Path) -> None:
        _unlink_if_exists(root, LINK_NAME)

    def prepare_echo_write(root: Path) -> None:
        _unlink_if_exists(root, ECHO_OUT)

    def prepare_mv(root: Path) -> None:
        backup = root / "bench_mv_backup.txt"
        if not backup.exists():
            shutil.copy2(root / SAMPLE, backup)
        shutil.copy2(backup, root / SAMPLE)
        _unlink_if_exists(root, MOVED_NAME)

    def prepare_sed_inplace(root: Path) -> None:
        _restore_sample(root)

    sed_inplace_native = (
        "sed -i '' 's/needle/replace/g' file_0001.txt"
        if sys.platform == "darwin"
        else ("sed -i 's/needle/replace/g' file_0001.txt")
    )

    rg_native: str | None = "rg needle ." if shutil.which("rg") else None

    return [
        BenchmarkCase("cat", f"cat {SAMPLE}", lambda _ws: CatCommand(path=SAMPLE)),
        BenchmarkCase("cd", "cd subdir", lambda _ws: CdCommand(path="subdir")),
        BenchmarkCase(
            "chmod",
            f"chmod 644 {SAMPLE}",
            lambda _ws: ChmodCommand(mode="644", path=SAMPLE),
        ),
        BenchmarkCase(
            "cp",
            f"cp {SAMPLE} {COPY_NAME}",
            lambda _ws: CopyCommand(src=SAMPLE, dst=COPY_NAME, overwrite=True),
            prepare=prepare_copy,
        ),
        BenchmarkCase("du", "du -s .", lambda _ws: DuCommand(path=".")),
        BenchmarkCase("echo", "echo benchmark", lambda _ws: EchoCommand(text="benchmark")),
        BenchmarkCase(
            "echo_write",
            f"echo benchmark > {ECHO_OUT}",
            lambda _ws: EchoCommand(text="benchmark", output_path=ECHO_OUT),
            prepare=prepare_echo_write,
        ),
        BenchmarkCase(
            "find",
            "find . -name 'file_*.txt'",
            lambda _ws: FindCommand(path=".", name="file_*.txt"),
        ),
        BenchmarkCase(
            "grep",
            "grep -r needle .",
            lambda _ws: GrepCommand(path=".", pattern="needle", recursive=True),
        ),
        BenchmarkCase(
            "head",
            f"head -n 5 {SAMPLE}",
            lambda _ws: HeadCommand(path=SAMPLE, lines=5),
        ),
        BenchmarkCase(
            "ln",
            f"ln -sf {SAMPLE} {LINK_NAME}",
            lambda _ws: LnCommand(src=SAMPLE, dst=LINK_NAME, symbolic=True, force=True),
            prepare=prepare_ln,
        ),
        BenchmarkCase("ls", "ls -la", lambda _ws: LsCommand(path=".", all=True, long=True)),
        BenchmarkCase(
            "mkdir",
            f"mkdir -p {MKDIR_PATH}",
            lambda _ws: MkdirCommand(path=MKDIR_PATH, parents=True),
            prepare=prepare_mkdir,
        ),
        BenchmarkCase(
            "mv",
            f"mv -f {SAMPLE} {MOVED_NAME}",
            lambda _ws: MoveCommand(src=SAMPLE, dst=MOVED_NAME, overwrite=True),
            prepare=prepare_mv,
        ),
        BenchmarkCase("nl", f"nl {SAMPLE}", lambda _ws: NlCommand(path=SAMPLE)),
        BenchmarkCase("pwd", "pwd", lambda _ws: PwdCommand()),
        BenchmarkCase(
            "rm",
            f"rm {COPY_NAME}",
            lambda _ws: RemoveCommand(path=COPY_NAME),
            prepare=prepare_remove,
        ),
        BenchmarkCase(
            "rg",
            rg_native,
            lambda _ws: RgCommand(pattern="needle", path="."),
            native_note=None if rg_native else "ripgrep not installed; native skipped",
        ),
        BenchmarkCase(
            "sed",
            f"sed 's/needle/replace/g' {SAMPLE}",
            lambda _ws: SedCommand(script="s/needle/replace/g", paths=[SAMPLE], quiet=False),
            prepare=prepare_sed_inplace,
        ),
        BenchmarkCase(
            "sed_inplace",
            sed_inplace_native,
            lambda _ws: SedCommand(
                script="s/needle/replace/g",
                paths=[SAMPLE],
                in_place=True,
            ),
            prepare=prepare_sed_inplace,
        ),
        BenchmarkCase("sort", f"sort {SAMPLE}", lambda _ws: SortCommand(path=SAMPLE)),
        BenchmarkCase("stat", f"stat {SAMPLE}", lambda _ws: StatCommand(path=SAMPLE)),
        BenchmarkCase(
            "tail",
            f"tail -n 5 {SAMPLE}",
            lambda _ws: TailCommand(path=SAMPLE, lines=5),
        ),
        BenchmarkCase(
            "touch",
            f"touch {TOUCH_PATH}",
            lambda _ws: TouchCommand(path=TOUCH_PATH),
            prepare=prepare_touch,
        ),
        BenchmarkCase(
            "wc",
            f"wc -l {SAMPLE}",
            lambda _ws: WcCommand(path=SAMPLE, lines=True),
        ),
    ]


def restore_baseline(root: Path) -> None:
    """Reset shared fixtures so each command benchmark starts from the same tree."""
    sample = root / SAMPLE
    backup = root / "bench_mv_backup.txt"
    if sample.exists() and not backup.exists():
        shutil.copy2(sample, backup)
    if backup.exists():
        shutil.copy2(backup, sample)
    _restore_sample(root)
    for relative in (COPY_NAME, MOVED_NAME, LINK_NAME, ECHO_OUT, TOUCH_PATH):
        _unlink_if_exists(root, relative)
    _unlink_if_exists(root, "bench_mkdir")


def prepare_workspace(root: Path, *, file_count: int, file_size: int) -> None:
    root.mkdir(parents=True, exist_ok=True)
    payload = ("needle line\n" * max(1, file_size // 12))[:file_size]
    for index in range(file_count):
        target = root / f"file_{index:04d}.txt"
        target.write_text(
            payload if index % 7 else f"needle special {index}\n{payload}",
            encoding="utf-8",
        )
    (root / "subdir").mkdir(exist_ok=True)
    (root / "subdir" / "nested.txt").write_text("nested needle\n", encoding="utf-8")
    shutil.copy2(root / SAMPLE, root / "bench_mv_backup.txt")
