from __future__ import annotations as _annotations

from pathlib import Path

import pytest

from vsh.session.resolver import (
    ensure_safe_workspace_root,
    get_protected_path_label,
    is_same_path_or_ancestor,
    is_within_workspace,
    resolve_workspace_path,
)
from vsh.session.state import SessionState


def test_resolve_workspace_path_supports_absolute_candidates(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    nested = workspace / "src"
    nested.mkdir()

    resolved = resolve_workspace_path(str(workspace), str(nested))

    assert resolved == str(nested.resolve())


def test_is_within_workspace_rejects_outside_paths(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    outside = tmp_path / "outside"
    outside.mkdir()

    assert is_within_workspace(str(outside), str(workspace)) is False


def test_is_same_path_or_ancestor_rejects_unrelated_paths(tmp_path: Path) -> None:
    first = tmp_path / "first"
    second = tmp_path / "second"
    first.mkdir()
    second.mkdir()

    assert is_same_path_or_ancestor(str(first), str(second)) is False


def test_get_protected_path_label_matches_static_roots() -> None:
    assert get_protected_path_label("/Applications") == "Applications directory"


def test_session_from_workspace_root_resolves_relative_cwd_against_workspace(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    nested = workspace / "src"
    nested.mkdir()
    other = tmp_path / "other"
    other.mkdir()
    monkeypatch.chdir(other)

    session = SessionState.from_workspace_root(str(workspace), cwd="src")

    assert session.cwd_logical == str(nested.resolve())
    assert is_within_workspace(session.cwd_logical, session.workspace_root)


def test_session_from_workspace_root_dot_cwd_is_workspace_not_process_cwd(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    other = tmp_path / "other"
    other.mkdir()
    monkeypatch.chdir(other)

    session = SessionState.from_workspace_root(str(workspace), cwd=".")

    assert session.cwd_logical == str(workspace.resolve())
    assert session.cwd_logical != str(other.resolve())


def test_session_state_with_cwd_updates_oldpwd(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    nested = workspace / "src"
    nested.mkdir()
    session = SessionState.from_workspace_root(str(workspace))

    updated = session.with_cwd(str(nested.resolve()))

    assert updated.oldpwd == session.cwd_logical
    assert updated.cwd_logical == str(nested.resolve())


def test_ensure_safe_workspace_root_rejects_home(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="workspace root is too broad or protected"):
        ensure_safe_workspace_root(str(Path.home()))
