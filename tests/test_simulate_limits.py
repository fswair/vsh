from __future__ import annotations as _annotations

from pathlib import Path

import pytest

from vsh.schemas import CurlCommand, GrepCommand, WgetCommand
from vsh.simulate.approval_levels import classify_approval_requirement, max_touched_paths
from vsh.simulate.engine import simulate_command
from vsh.simulate.models import Overlay
from vsh.simulate.protected_paths import (
    DEFAULT_PROTECTED_PATTERNS,
    _match_globstar_pattern,
    clear_protected_patterns_cache,
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


def test_classify_approval_requirement_for_network_commands() -> None:
    curl_stdout_tier, curl_stdout_manual = classify_approval_requirement(
        CurlCommand(url="https://example.com"),
        decision="approve",
    )
    assert curl_stdout_tier == "read_only"
    assert curl_stdout_manual is True

    curl_output_tier, curl_output_manual = classify_approval_requirement(
        CurlCommand(url="https://example.com", output_path="page.html"),
        decision="approve",
    )
    assert curl_output_tier == "mutation"
    assert curl_output_manual is True

    wget_tier, wget_manual = classify_approval_requirement(
        WgetCommand(url="https://example.com/readme.txt"),
        decision="approve",
    )
    assert wget_tier == "mutation"
    assert wget_manual is True


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
    clear_protected_patterns_cache()
    monkeypatch.setenv("VSH_PROTECTED_PATTERNS", "*.secret,private.key")
    assert load_protected_patterns() == ("*.secret", "private.key")

    clear_protected_patterns_cache()
    monkeypatch.delenv("VSH_PROTECTED_PATTERNS", raising=False)
    patterns_file = tmp_path / "patterns.txt"
    patterns_file.write_text("# comment\n*.vault\n", encoding="utf-8")
    monkeypatch.setenv("VSH_PROTECTED_PATTERNS_FILE", str(patterns_file))
    assert load_protected_patterns() == ("*.vault",)


def test_load_protected_patterns_falls_back_when_env_is_blank(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    clear_protected_patterns_cache()
    monkeypatch.setenv("VSH_PROTECTED_PATTERNS", " , ")
    assert load_protected_patterns() == DEFAULT_PROTECTED_PATTERNS


def test_load_protected_patterns_falls_back_when_file_has_only_comments(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    clear_protected_patterns_cache()
    patterns_file = tmp_path / "patterns.txt"
    patterns_file.write_text("# only comments\n", encoding="utf-8")
    monkeypatch.setenv("VSH_PROTECTED_PATTERNS_FILE", str(patterns_file))
    assert load_protected_patterns() == DEFAULT_PROTECTED_PATTERNS


def test_load_protected_patterns_ignores_missing_file_mtime(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    clear_protected_patterns_cache()
    monkeypatch.setenv("VSH_PROTECTED_PATTERNS_FILE", "/tmp/does-not-exist-patterns.txt")
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
