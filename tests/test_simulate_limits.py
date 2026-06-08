from __future__ import annotations as _annotations

from pathlib import Path

import pytest

from vsh.schemas import GrepCommand
from vsh.simulate.approval_levels import classify_approval_requirement, max_touched_paths
from vsh.simulate.engine import simulate_command
from vsh.simulate.models import Overlay
from vsh.simulate.protected_paths import (
    DEFAULT_PROTECTED_PATTERNS,
    _match_globstar_pattern,
    load_protected_patterns,
    matches_protected_pattern,
)
from vsh.snapshot.builder import snapshot_workspace


def test_max_touched_paths_rejects_large_read_scope(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    for index in range(5):
        (workspace / f"file{index}.txt").write_text("x\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    monkeypatch.setenv("VSH_MAX_TOUCHED_PATHS", "2")

    result = simulate_command(GrepCommand(pattern="x", path=".", recursive=True), snapshot)

    assert result.decision == "reject"
    assert result.reason is not None
    assert "too many paths" in result.reason


def test_max_touched_paths_invalid_env_falls_back_to_default(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("VSH_MAX_TOUCHED_PATHS", "not-a-number")
    assert max_touched_paths() == 500


def test_classify_approval_requirement_for_rejected_plan() -> None:
    from vsh.schemas import LsCommand

    tier, manual = classify_approval_requirement(LsCommand(path="."), decision="reject")
    assert tier == "read_only"
    assert manual is True


def test_classify_approval_requirement_for_overlay_delete() -> None:
    from vsh.schemas import TouchCommand

    tier, manual = classify_approval_requirement(
        TouchCommand(path="x.txt"),
        decision="approve_with_warning",
        overlay=Overlay(deleted={"x"}),
    )
    assert tier == "destructive"
    assert manual is True


def test_load_protected_patterns_from_env_and_file(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("VSH_PROTECTED_PATTERNS", "*.secret,private.key")
    assert load_protected_patterns() == ("*.secret", "private.key")

    monkeypatch.delenv("VSH_PROTECTED_PATTERNS", raising=False)
    patterns_file = tmp_path / "patterns.txt"
    patterns_file.write_text("# comment\n*.vault\n", encoding="utf-8")
    monkeypatch.setenv("VSH_PROTECTED_PATTERNS_FILE", str(patterns_file))
    assert load_protected_patterns() == ("*.vault",)


def test_load_protected_patterns_falls_back_when_env_is_blank(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setenv("VSH_PROTECTED_PATTERNS", " , ")
    assert load_protected_patterns() == DEFAULT_PROTECTED_PATTERNS


def test_load_protected_patterns_falls_back_when_file_has_only_comments(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    patterns_file = tmp_path / "patterns.txt"
    patterns_file.write_text("# only comments\n", encoding="utf-8")
    monkeypatch.setenv("VSH_PROTECTED_PATTERNS_FILE", str(patterns_file))
    assert load_protected_patterns() == DEFAULT_PROTECTED_PATTERNS


def test_matches_protected_pattern_uses_basename_fnmatch() -> None:
    assert matches_protected_pattern("nested/foo.pem", ("*.pem",)) is True


def test_match_globstar_suffix_and_fnmatch_fallback() -> None:
    assert _match_globstar_pattern("nested/id_rsa", "**/id_rsa") is True
    assert _match_globstar_pattern("foo/bar", "**/bar") is True
    assert matches_protected_pattern("deploy/id_rsa.pub", ("**/id_rsa.pub",)) is True
    assert matches_protected_pattern("server.pem", ("*.pem",)) is True
    assert matches_protected_pattern("pkg/.env.local", (".env.*",)) is True
    assert _match_globstar_pattern("a/foo_bar", "**/foo*") is True
    assert _match_globstar_pattern("foo/bar/baz", "**/bar/**") is True
