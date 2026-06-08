from __future__ import annotations as _annotations

from pathlib import Path

from vsh.schemas import CatCommand, CopyCommand, RemoveCommand
from vsh.simulate.engine import simulate_command
from vsh.simulate.protected_paths import (
    DEFAULT_PROTECTED_PATTERNS,
    _match_globstar_pattern,
    get_protected_workspace_path_reason,
    matches_protected_pattern,
)
from vsh.snapshot.builder import snapshot_workspace


def test_matches_protected_pattern_for_env_files() -> None:
    assert matches_protected_pattern(".env", DEFAULT_PROTECTED_PATTERNS)
    assert matches_protected_pattern("apps/api/.env.local", DEFAULT_PROTECTED_PATTERNS)
    assert matches_protected_pattern("deploy/id_rsa", DEFAULT_PROTECTED_PATTERNS)
    assert matches_protected_pattern("nested/foo.pem", ("*.pem",))
    assert not matches_protected_pattern("main.py", ("*.pem",))
    assert matches_protected_pattern("token.secret", ("*.secret",))
    assert not matches_protected_pattern("src/main.py", DEFAULT_PROTECTED_PATTERNS)


def test_match_globstar_fnmatch_fallback() -> None:
    assert _match_globstar_pattern("a/foo_bar", "**/foo*") is True
    assert _match_globstar_pattern("plain", "**/missing") is False
    assert _match_globstar_pattern("foo/bar", "**/bar") is True
    assert _match_globstar_pattern("secrets/token", "secrets/**") is True
    assert _match_globstar_pattern("foo/mid/bar", "foo/**/bar") is True
    assert _match_globstar_pattern("x", "literal/path") is False
    assert not matches_protected_pattern("src/main.py", ("exact/nested/path",))


def test_simulate_rejects_reading_protected_env_file(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    env_file = workspace / ".env"
    env_file.write_text("SECRET=1\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    result = simulate_command(CatCommand(path=".env"), snapshot)

    assert result.decision == "reject"
    assert result.reason is not None
    assert "protected workspace pattern" in result.reason


def test_simulate_rejects_read_inside_protected_secrets_tree(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    secrets = workspace / "secrets"
    secrets.mkdir()
    (secrets / "token.txt").write_text("x\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    result = simulate_command(CatCommand(path="secrets/token.txt"), snapshot)

    assert result.decision == "reject"
    assert result.reason is not None


def test_simulate_rejects_mutation_into_protected_path(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    result = simulate_command(CopyCommand(src="missing.txt", dst=".env"), snapshot)

    assert result.decision == "reject"
    assert result.reason is not None
    assert "protected workspace pattern" in result.reason


def test_get_protected_workspace_path_reason_ignores_outside_paths(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    outside = tmp_path / ".env"
    outside.write_text("SECRET=1\n", encoding="utf-8")

    assert get_protected_workspace_path_reason(str(outside), str(workspace)) is None


def test_remove_into_protected_file_is_rejected(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    key = workspace / "server.pem"
    key.write_text("pem\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    result = simulate_command(RemoveCommand(path="server.pem"), snapshot)

    assert result.decision == "reject"
    assert result.reason is not None
