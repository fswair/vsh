from __future__ import annotations as _annotations

import os
from pathlib import Path

import pytest

from vsh.execute.dispatch import ExecutionContext, apply_command
from vsh.execute.read_output import _human_size, capture_read_output
from vsh.schemas import (
    CatCommand,
    DuCommand,
    FindCommand,
    GrepCommand,
    HeadCommand,
    LsCommand,
    NlCommand,
    PwdCommand,
    RgCommand,
    SedCommand,
    SortCommand,
    StatCommand,
    TailCommand,
    WcCommand,
)


def _ctx(workspace: Path) -> ExecutionContext:
    return ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))


def _stdout(stdout: str | None) -> str:
    assert stdout is not None
    return stdout


def test_apply_command_returns_stdout_for_cat(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "notes.txt"
    target.write_text("alpha\nbeta\n", encoding="utf-8")

    effects = apply_command(CatCommand(path="notes.txt"), _ctx(workspace))

    assert effects.stdout == "alpha\nbeta\n"
    assert str(target.resolve()) in effects.reads


def test_cat_supports_numbering_and_show_ends(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "tab.txt"
    target.write_text("a\tb\n", encoding="utf-8")

    effects = apply_command(
        CatCommand(path="tab.txt", number=True, show_ends=True, squeeze_blank=False),
        _ctx(workspace),
    )

    assert "1\t" in _stdout(effects.stdout)
    assert "$" in _stdout(effects.stdout)


def test_apply_command_returns_stdout_for_head_and_wc(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "data.txt"
    target.write_text("one two\nthree\n", encoding="utf-8")

    head = apply_command(HeadCommand(path="data.txt", lines=1), _ctx(workspace))
    wc = apply_command(WcCommand(path="data.txt", lines=True, words=True), _ctx(workspace))

    assert head.stdout == "one two\n"
    assert "2" in _stdout(wc.stdout)
    assert "data.txt" in _stdout(wc.stdout)


def test_apply_command_returns_stdout_for_ls_and_grep(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "alpha.txt").write_text("hello\n", encoding="utf-8")
    (workspace / "beta.txt").write_text("world\n", encoding="utf-8")

    ls = apply_command(LsCommand(path=".", long=True), _ctx(workspace))
    grep = apply_command(GrepCommand(pattern="hello", path=".", line_number=True), _ctx(workspace))

    assert "alpha.txt" in _stdout(ls.stdout)
    assert "beta.txt" in _stdout(ls.stdout)
    assert "hello" in _stdout(grep.stdout)


def test_ls_recursive_and_hidden_entries(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    nested = workspace / "nested"
    nested.mkdir()
    (nested / "child.txt").write_text("x\n", encoding="utf-8")
    (workspace / ".hidden").write_text("secret\n", encoding="utf-8")

    effects = apply_command(LsCommand(path=".", all=True, recursive=True), _ctx(workspace))

    assert ".hidden" in _stdout(effects.stdout)
    assert "nested/child.txt" in _stdout(effects.stdout)


def test_tail_follow_is_rejected(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "log.txt"
    target.write_text("line\n", encoding="utf-8")

    with pytest.raises(ValueError, match="tail --follow"):
        apply_command(TailCommand(path="log.txt", follow=True), _ctx(workspace))


def test_sort_nl_stat_du_and_find_outputs(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "b.txt").write_text("b\n", encoding="utf-8")
    (workspace / "a.txt").write_text("a\n", encoding="utf-8")
    nested = workspace / "pkg"
    nested.mkdir()

    sort = apply_command(SortCommand(path="b.txt", unique=True, reverse=True), _ctx(workspace))
    nl = apply_command(NlCommand(path="a.txt", number_all=False), _ctx(workspace))
    stat = apply_command(StatCommand(path="a.txt"), _ctx(workspace))
    du = apply_command(DuCommand(path=".", human_readable=True, summarize=True), _ctx(workspace))
    find = apply_command(FindCommand(path=".", name="*.txt", type="file"), _ctx(workspace))

    assert sort.stdout == "b\n"
    assert "1\t" in _stdout(nl.stdout)
    assert "File:" in _stdout(stat.stdout)
    assert "B" in _stdout(du.stdout) or "K" in _stdout(du.stdout)
    assert "a.txt" in _stdout(find.stdout)


def test_wc_defaults_and_sed_substitute_read_mode(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "sample.txt"
    target.write_text("old value\n", encoding="utf-8")

    wc = apply_command(WcCommand(path="sample.txt"), _ctx(workspace))
    sed = apply_command(
        SedCommand(script="s/old/new/g", paths=["sample.txt"], in_place=False),
        _ctx(workspace),
    )

    assert "sample.txt" in _stdout(wc.stdout)
    assert "new value" in _stdout(sed.stdout)


def test_rg_hidden_and_grep_fixed_strings(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / ".secret").write_text("needle\n", encoding="utf-8")

    rg = apply_command(RgCommand(pattern="needle", path=".", hidden=True), _ctx(workspace))
    grep = apply_command(
        GrepCommand(pattern="needle", path=".secret", fixed_strings=True),
        _ctx(workspace),
    )

    assert "needle" in _stdout(rg.stdout)
    assert "needle" in _stdout(grep.stdout)


def test_capture_read_output_rejects_unsupported_command(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    with pytest.raises(ValueError, match="unsupported read command"):
        capture_read_output(PwdCommand(), _ctx(workspace))


def test_human_size_renders_large_units() -> None:
    assert _human_size(2048).endswith("K")
    assert _human_size(10).endswith("B")
    assert _human_size(1024**5).endswith("T")
    assert _human_size(1024**3).endswith("G")
    assert _human_size(1024**4).endswith("T")


def test_ls_rejects_non_directory_targets(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "file.txt"
    target.write_text("x\n", encoding="utf-8")

    with pytest.raises(ValueError, match="ls target is not a directory"):
        apply_command(LsCommand(path="file.txt"), _ctx(workspace))


def test_cat_squeeze_blank_and_files_without_trailing_newline(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "sparse.txt"
    target.write_text("a\n\nb", encoding="utf-8")

    effects = apply_command(CatCommand(path="sparse.txt", squeeze_blank=True), _ctx(workspace))

    assert "a\nb" in _stdout(effects.stdout).replace("\n\n", "\n")


def test_nl_preserves_unnumbered_blank_lines(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "blank.txt"
    target.write_text("line\n\n", encoding="utf-8")

    effects = apply_command(NlCommand(path="blank.txt", number_all=False), _ctx(workspace))

    assert _stdout(effects.stdout).startswith("     1\tline")


def test_find_respects_maxdepth_and_directory_filter(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    nested = workspace / "nested"
    nested.mkdir()

    effects = apply_command(
        FindCommand(path=".", type="dir", maxdepth=1),
        _ctx(workspace),
    )

    assert any("nested" in line for line in _stdout(effects.stdout).splitlines())


def test_head_and_tail_on_empty_files_return_no_stdout(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "empty.txt"
    target.write_text("", encoding="utf-8")

    head = apply_command(HeadCommand(path="empty.txt", lines=3), _ctx(workspace))
    tail = apply_command(TailCommand(path="empty.txt", lines=3), _ctx(workspace))

    assert head.stdout == ""
    assert tail.stdout == ""


def test_wc_chars_and_bytes_flags(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "chars.txt"
    target.write_text("abc", encoding="utf-8")

    effects = apply_command(WcCommand(path="chars.txt", chars=True, bytes=True), _ctx(workspace))

    assert "3" in _stdout(effects.stdout)


def test_sort_and_nl_on_empty_files(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "empty.txt"
    target.write_text("", encoding="utf-8")

    sort = apply_command(SortCommand(path="empty.txt"), _ctx(workspace))
    nl = apply_command(NlCommand(path="empty.txt"), _ctx(workspace))

    assert sort.stdout == ""
    assert nl.stdout == ""


def test_sort_unique_collapses_duplicate_lines(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "dup.txt"
    target.write_text("b\na\na\n", encoding="utf-8")

    effects = apply_command(SortCommand(path="dup.txt", unique=True), _ctx(workspace))

    assert effects.stdout == "a\nb\n"


def test_find_filters_by_name_and_node_type(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    source = workspace / "source.txt"
    source.write_text("x\n", encoding="utf-8")
    nested = workspace / "nested"
    nested.mkdir()

    by_name = apply_command(FindCommand(path=".", name="*.txt", type="file"), _ctx(workspace))
    by_dir = apply_command(FindCommand(path=".", type="dir", maxdepth=1), _ctx(workspace))

    assert "source.txt" in _stdout(by_name.stdout)
    assert "nested" in _stdout(by_dir.stdout)


def test_find_can_match_symlink_entries(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "source.txt").write_text("x\n", encoding="utf-8")
    link = workspace / "link.txt"

    effects = _run_find_symlink_test(workspace)

    assert str(link.resolve()) in _stdout(effects.stdout)
    assert len(effects.reads) >= 2


def test_find_skips_when_symlink_creation_unavailable(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "source.txt").write_text("x\n", encoding="utf-8")

    def fail_symlink(*_args: object, **_kwargs: object) -> None:
        raise OSError("symlink unavailable")

    monkeypatch.setattr(os, "symlink", fail_symlink)

    with pytest.raises(pytest.skip.Exception, match="symlink creation unavailable"):
        _run_find_symlink_test(workspace)


def test_find_skips_when_symlink_entry_is_not_created(
    monkeypatch: pytest.MonkeyPatch,
    tmp_path: Path,
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "source.txt").write_text("x\n", encoding="utf-8")
    link = workspace / "link.txt"
    os.symlink("source.txt", link)
    monkeypatch.setattr(Path, "is_symlink", lambda _self: False)

    with pytest.raises(pytest.skip.Exception, match="platform did not create a symlink entry"):
        _run_find_symlink_test(workspace)


def _run_find_symlink_test(workspace: Path):
    link = workspace / "link.txt"
    if not link.exists():
        try:
            os.symlink("source.txt", link)
        except OSError:
            pytest.skip("symlink creation unavailable on this platform")
    if not link.is_symlink():
        pytest.skip("platform did not create a symlink entry")

    return apply_command(FindCommand(path=".", type="symlink"), _ctx(workspace))


def test_iter_files_skips_hidden_entries(tmp_path: Path) -> None:
    from vsh.execute.read_output import _iter_files

    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / ".only").write_text("needle\n", encoding="utf-8")
    (workspace / "subdir").mkdir()

    files = _iter_files(workspace, recursive=False, include_hidden=False)

    assert files == []


def test_grep_non_recursive_skips_hidden_entries(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / ".hidden").write_text("secret\n", encoding="utf-8")
    (workspace / "visible.txt").write_text("needle\n", encoding="utf-8")

    effects = apply_command(GrepCommand(pattern="needle", path="."), _ctx(workspace))

    assert "needle" in _stdout(effects.stdout)
    assert ".hidden" not in _stdout(effects.stdout)


def test_grep_skips_unreadable_files_and_supports_single_file_paths(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    readable = workspace / "ok.txt"
    readable.write_text("needle\n", encoding="utf-8")
    binary = workspace / "bad.bin"
    binary.write_bytes(b"\xff\xfe\x00\x00")

    single = apply_command(GrepCommand(pattern="needle", path="ok.txt"), _ctx(workspace))
    skipped = apply_command(
        GrepCommand(pattern="needle", path=".", recursive=True), _ctx(workspace)
    )

    assert "needle" in _stdout(single.stdout)
    assert "needle" in _stdout(skipped.stdout)


def test_du_on_single_file_uses_file_size(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "solo.txt"
    target.write_text("12345", encoding="utf-8")

    effects = apply_command(DuCommand(path="solo.txt"), _ctx(workspace))

    assert "solo.txt" in _stdout(effects.stdout)


def test_sed_read_output_joins_multiple_files(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "a.txt").write_text("old", encoding="utf-8")
    (workspace / "b.txt").write_text("old", encoding="utf-8")

    effects = apply_command(
        SedCommand(script="s/old/new/g", paths=["a.txt", "b.txt"], in_place=False),
        _ctx(workspace),
    )

    assert "new" in _stdout(effects.stdout)


def test_find_type_file_skips_directories_and_empty_results(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    nested = workspace / "nested"
    nested.mkdir()

    only_dirs = apply_command(FindCommand(path=".", type="file"), _ctx(workspace))
    missing = apply_command(FindCommand(path=".", name="*.missing"), _ctx(workspace))

    assert only_dirs.stdout == ""
    assert missing.stdout == ""


def test_sed_unsupported_script_raises(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "x.txt"
    target.write_text("x\n", encoding="utf-8")

    with pytest.raises(ValueError, match="unsupported sed script"):
        apply_command(
            SedCommand(script="invalid", paths=["x.txt"], in_place=False), _ctx(workspace)
        )


def test_read_commands_require_existing_paths(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    with pytest.raises(FileNotFoundError, match="path does not exist"):
        apply_command(CatCommand(path="missing.txt"), _ctx(workspace))


def test_sed_print_script_handles_missing_line_numbers(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "short.txt"
    target.write_text("only\n", encoding="utf-8")

    empty = apply_command(
        SedCommand(script="9p", paths=["short.txt"], in_place=False),
        _ctx(workspace),
    )
    printed = apply_command(
        SedCommand(script="1p", paths=["short.txt"], in_place=False),
        _ctx(workspace),
    )

    assert empty.stdout == ""
    assert printed.stdout == "only\n"
