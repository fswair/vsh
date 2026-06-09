from __future__ import annotations as _annotations

import signal
import socket
import subprocess
from collections.abc import Callable
from pathlib import Path
from typing import Any, cast
from unittest.mock import MagicMock

import httpx
import pytest

from vsh.errors import DriftError, PolicyRejection, ValidationFailure, VshError
from vsh.execute.dispatch import ExecutionContext, apply_command
from vsh.execute.git_commands import run_git_diff, run_git_status
from vsh.execute.patch import apply_patch_to_file
from vsh.execute.rollback import RollbackSession, restore_session
from vsh.http.fetch import fetch_http
from vsh.http.ssrf import SsrfBlockedError, validate_outbound_url
from vsh.limits import (
    compile_grep_pattern,
    grep_max_file_bytes,
    grep_max_matches,
    grep_regex_timeout_secs,
    read_max_file_bytes,
)
from vsh.mcp import tools
from vsh.mcp.receipts import ApplyReceipt, BatchReceipt
from vsh.policy.profiles import load_policy_profile
from vsh.schemas import ApplyPatchCommand, GitDiffCommand, GitStatusCommand, GrepCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import content_hash_enabled, snapshot_workspace
from vsh.snapshot.cache import SnapshotCache, cache_enabled, snapshot_age_seconds, snapshot_cache
from vsh.snapshot.ignore import SnapshotIgnoreMatcher


def test_error_types_expose_codes() -> None:
    assert ValidationFailure("x").error_code == "validation_error"
    assert PolicyRejection("x", hint="h").hint == "h"
    assert DriftError("x").error_code == "drift_stale"
    assert isinstance(VshError("x"), VshError)


def test_apply_patch_errors(tmp_path: Path) -> None:
    target = tmp_path / "a.txt"
    target.write_text("one\n", encoding="utf-8")
    with pytest.raises(ValueError, match="search-replace"):
        apply_patch_to_file(
            ApplyPatchCommand(path="a.txt", patch="only-old", execution_reason="x"),
            str(target),
        )
    with pytest.raises(ValueError, match="not found"):
        apply_patch_to_file(
            ApplyPatchCommand(path="a.txt", patch="missing\n===\nnew\n", execution_reason="x"),
            str(target),
        )


def test_git_commands_require_repo(tmp_path: Path) -> None:
    with pytest.raises(ValueError, match="not a git repository"):
        run_git_status(str(tmp_path))


def test_git_commands_run_in_repo(tmp_path: Path) -> None:
    subprocess.run(["git", "init"], cwd=tmp_path, check=True, capture_output=True)
    (tmp_path / "a.txt").write_text("x\n", encoding="utf-8")
    subprocess.run(["git", "add", "a.txt"], cwd=tmp_path, check=True, capture_output=True)
    status = run_git_status(str(tmp_path))
    assert "a.txt" in status
    diff = run_git_diff(GitDiffCommand(path=".", staged=True), str(tmp_path))
    assert "a.txt" in diff or diff == ""


def test_rollback_session_restore(tmp_path: Path) -> None:
    target = tmp_path / "file.txt"
    target.write_text("before\n", encoding="utf-8")
    session = RollbackSession(backup_root=tmp_path / "backup")
    session.record(target, existed=True)
    target.write_text("after\n", encoding="utf-8")
    restore_session(session)
    assert target.read_text(encoding="utf-8") == "before\n"


def test_rollback_session_restore_created_file(tmp_path: Path) -> None:
    target = tmp_path / "new.txt"
    session = RollbackSession(backup_root=tmp_path / "backup")
    session.record(target, existed=False)
    target.write_text("after\n", encoding="utf-8")
    restore_session(session)
    assert not target.exists()


def test_rollback_restore_missing_created_entry(tmp_path: Path) -> None:
    target = tmp_path / "never-created.txt"
    session = RollbackSession(backup_root=tmp_path / "backup")
    session.record(target, existed=False)
    restore_session(session)
    assert not target.exists()


def test_fetch_http_follows_redirect(monkeypatch: pytest.MonkeyPatch) -> None:
    calls: list[str] = []

    class _Client:
        def __init__(self, *args: object, **kwargs: object) -> None:
            pass

        def __enter__(self) -> _Client:
            return self

        def __exit__(self, *args: object) -> None:
            return None

        def request(self, method: str, url: str, **kwargs: object) -> httpx.Response:
            calls.append(url)
            if len(calls) == 1:
                return httpx.Response(
                    302,
                    headers={"location": "https://example.com/final"},
                    request=httpx.Request("GET", url),
                )
            return httpx.Response(200, text="ok", request=httpx.Request("GET", url))

    monkeypatch.setattr("vsh.http.fetch.httpx.Client", _Client)
    result = fetch_http(url="https://example.com/start", max_bytes=100)
    assert result.stdout == "ok"
    assert len(calls) == 2


def test_ssrf_allowed_hosts_env(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("VSH_HTTP_ALLOWED_HOSTS", "allowed.test")
    with pytest.raises(SsrfBlockedError):
        validate_outbound_url("https://example.com")
    assert validate_outbound_url("https://allowed.test/x") == "https://allowed.test/x"


def test_limits_helpers(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("VSH_READ_MAX_BYTES", "not-int")
    monkeypatch.setenv("VSH_GREP_MAX_FILE_BYTES", "bad")
    monkeypatch.setenv("VSH_GREP_MAX_MATCHES", "bad")
    monkeypatch.setenv("VSH_GREP_REGEX_TIMEOUT_SECS", "bad")
    assert read_max_file_bytes() == 1_048_576
    assert grep_max_file_bytes() == 1_048_576
    assert grep_max_matches() == 10_000
    assert grep_regex_timeout_secs() == 2.0
    pattern = compile_grep_pattern(
        "needle", ignore_case=False, fixed_strings=True, extended_regexp=False
    )
    assert pattern is None


def test_receipt_models() -> None:
    receipt = ApplyReceipt(status="applied", tool_name="vsh_list")
    assert receipt.status == "applied"
    batch = BatchReceipt(status="ok", snapshot_id="snap_x", completed_steps=1, steps=[])
    assert batch.completed_steps == 1


def test_apply_batch_transactional_rolls_back(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    result = tools.apply_batch(
        [
            {
                "tool_name": "vsh_mkdir",
                "params": {"path": "bench"},
                "execution_reason": "create",
            },
            {"tool_name": "vsh_missing", "params": {}},
        ],
        workspace_root=str(workspace),
        transactional=True,
    )
    assert result["status"] == "error"
    assert not (workspace / "bench").exists()


def test_grep_alias_non_string_does_not_set_path() -> None:
    from vsh.mcp.tools import _normalize_apply_params

    params = _normalize_apply_params("vsh_grep", {"root_directory": 1, "pattern": "x"})
    assert "path" not in params
    assert "root_directory" not in params


def test_apply_batch_transactional_and_aliases(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    result = tools.apply_batch(
        [
            {
                "tool_name": "vsh_mkdir",
                "params": {"directory": "out"},
                "execution_reason": "create",
            },
            {
                "tool_name": "vsh_echo",
                "params": {"file_path": "out/a.txt", "content": "hi\\n"},
                "execution_reason": "write",
            },
        ],
        workspace_root=str(workspace),
    )
    assert result["status"] == "ok"


def test_policy_invalid_preset(tmp_path: Path) -> None:
    (tmp_path / "vsh.toml").write_text('preset = "missing"\n', encoding="utf-8")
    with pytest.raises(ValueError, match="unknown policy preset"):
        load_policy_profile(tmp_path)


def test_snapshot_cache_and_ignore(tmp_path: Path) -> None:
    (tmp_path / ".gitignore").write_text("ignored/\n", encoding="utf-8")
    (tmp_path / "ignored").mkdir()
    (tmp_path / "ignored" / "skip.txt").write_text("x", encoding="utf-8")
    (tmp_path / "keep.txt").write_text("y", encoding="utf-8")
    matcher = SnapshotIgnoreMatcher(workspace_root=tmp_path)
    assert matcher.is_ignored(tmp_path / "ignored", is_dir=True)
    snapshot = snapshot_workspace(str(tmp_path), cwd=str(tmp_path))
    assert str((tmp_path / "keep.txt").resolve()) in snapshot.nodes
    cache = SnapshotCache()
    cache.put(snapshot)
    assert cache.get(str(tmp_path)) is not None
    cache.clear()
    assert cache.get(str(tmp_path)) is None
    assert snapshot_age_seconds(snapshot) >= 0.0
    assert isinstance(cache_enabled(), bool)


def test_content_hash_env(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setenv("VSH_CONTENT_HASH", "1")
    assert content_hash_enabled() is True
    (tmp_path / "f.txt").write_text("hash-me\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(tmp_path), cwd=str(tmp_path))
    node = snapshot.nodes[str(tmp_path / "f.txt")]
    assert node.content_ref is not None
    assert node.content_ref.startswith("hash:blake2b:")


def test_grep_non_recursive_simulation(tmp_path: Path) -> None:
    root = tmp_path / "workspace"
    (root / "sub").mkdir(parents=True)
    (root / "sub" / "a.txt").write_text("needle\n", encoding="utf-8")
    (root / "b.txt").write_text("none\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(root), cwd=str(root))
    result = simulate_command(GrepCommand(pattern="needle", path=".", recursive=False), snapshot)
    assert result.decision != "reject"


def test_git_and_patch_simulation(tmp_path: Path) -> None:
    subprocess.run(["git", "init"], cwd=tmp_path, check=True, capture_output=True)
    snapshot = snapshot_workspace(str(tmp_path), cwd=str(tmp_path))
    status = simulate_command(GitStatusCommand(path="."), snapshot)
    assert status.decision != "reject"
    patch = simulate_command(
        ApplyPatchCommand(path="a.txt", patch="old\n===\nnew\n", execution_reason="patch"),
        snapshot,
    )
    assert patch.approval_tier == "mutation"


def test_grep_execution_limits(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    big = workspace / "big.txt"
    big.write_text("x" * 50 + "\n", encoding="utf-8")
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))
    command = GrepCommand(pattern="x", path="big.txt", recursive=False)
    effects = apply_command(command, ctx)
    assert "x" in (effects.stdout or "")


def test_dispatch_git_and_patch_create(tmp_path: Path) -> None:
    subprocess.run(["git", "init"], cwd=tmp_path, check=True, capture_output=True)
    ctx = ExecutionContext(workspace_root=str(tmp_path), cwd_logical=str(tmp_path))
    status = apply_command(GitStatusCommand(path="."), ctx)
    assert status.stdout is not None
    diff = apply_command(GitDiffCommand(path=".", staged=False), ctx)
    assert diff.stdout is not None
    (tmp_path / "new.txt").write_text("old\n", encoding="utf-8")
    patch = ApplyPatchCommand(
        path="new.txt",
        patch="old\n===\ncontent\n",
        execution_reason="update file",
    )
    updated = apply_command(patch, ctx)
    assert updated.updates


def test_snapshot_cache_hit(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setenv("VSH_SNAPSHOT_CACHE", "1")
    monkeypatch.setenv("VSH_SNAPSHOT_CACHE_MAX_AGE_SECS", "3600")
    first = snapshot_workspace(str(tmp_path), cwd=str(tmp_path))
    second = snapshot_workspace(str(tmp_path), cwd=str(tmp_path))
    assert first.snapshot_id == second.snapshot_id


def test_snapshot_cache_miss_rebuilds(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setenv("VSH_SNAPSHOT_CACHE", "1")
    monkeypatch.setenv("VSH_SNAPSHOT_CACHE_MAX_AGE_SECS", "0")
    (tmp_path / "a.txt").write_text("a\n", encoding="utf-8")
    first = snapshot_workspace(str(tmp_path), cwd=str(tmp_path))
    second = snapshot_workspace(str(tmp_path), cwd=str(tmp_path))
    assert first.snapshot_id != second.snapshot_id


def test_snapshot_cache_disabled_skips_store(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    monkeypatch.setenv("VSH_SNAPSHOT_CACHE", "0")
    snapshot = snapshot_workspace(str(tmp_path), cwd=str(tmp_path))
    assert snapshot.snapshot_id.startswith("snap_")
    assert snapshot_cache.get(str(tmp_path)) is None


def test_snapshot_builder_skips_non_file_entries(tmp_path: Path) -> None:
    import os

    from vsh.snapshot import builder

    fifo = tmp_path / "pipe"
    os.mkfifo(fifo)
    nodes = builder._build_nodes(tmp_path)  # noqa: SLF001
    assert str(tmp_path) in nodes
    assert str(fifo) not in nodes


def test_snapshot_builder_file_root(tmp_path: Path) -> None:
    from vsh.snapshot import builder

    file_root = tmp_path / "solo.txt"
    file_root.write_text("solo\n", encoding="utf-8")
    nodes = builder._build_nodes(file_root)  # noqa: SLF001
    assert str(file_root) in nodes


def test_snapshot_builder_skips_ignored_file(tmp_path: Path) -> None:
    from vsh.snapshot import builder

    (tmp_path / ".gitignore").write_text("skip.txt\n", encoding="utf-8")
    (tmp_path / "skip.txt").write_text("hidden\n", encoding="utf-8")
    (tmp_path / "keep.txt").write_text("keep\n", encoding="utf-8")
    nodes = builder._build_nodes(tmp_path)  # noqa: SLF001
    assert str(tmp_path / "keep.txt") in nodes
    assert str(tmp_path / "skip.txt") not in nodes


def test_cat_read_limit(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("VSH_READ_MAX_BYTES", "4")
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "big.txt").write_bytes(b"12345")
    from vsh.schemas import CatCommand

    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))
    with pytest.raises(ValueError, match="max bytes"):
        apply_command(CatCommand(path="big.txt"), ctx)


def test_grep_max_matches_and_file_size(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("VSH_GREP_MAX_MATCHES", "1")
    monkeypatch.setenv("VSH_GREP_MAX_FILE_BYTES", "1")
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "a.txt").write_text("needle\nneedle\n", encoding="utf-8")
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))
    effects = apply_command(GrepCommand(pattern="needle", path=".", recursive=True), ctx)
    assert effects.stdout is not None


def test_rollback_directory_backup(tmp_path: Path) -> None:
    src = tmp_path / "dir"
    src.mkdir()
    (src / "inner.txt").write_text("x\n", encoding="utf-8")
    session = RollbackSession(backup_root=tmp_path / "backup")
    session.record(src, existed=True)
    import shutil

    shutil.rmtree(src)
    restore_session(session)
    assert (src / "inner.txt").read_text(encoding="utf-8") == "x\n"


def test_validation_error_hints(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    receipt = tools.apply(
        "vsh_mkdir",
        {"directory": 123},
        workspace_root=str(workspace),
        cwd=".",
        execution_reason="x",
    )
    assert receipt["status"] == "error"
    assert receipt.get("hint") is not None


def test_classify_policy_and_drift_errors() -> None:
    from vsh.mcp.tools import _classify_apply_error

    code, hint = _classify_apply_error(None, tool_name="x", reason="plan is stale")
    assert code == "drift_stale"
    code2, _ = _classify_apply_error(None, tool_name="x", reason="policy reject")
    assert code2 == "policy_reject"


def test_fetch_http_without_following_redirects(monkeypatch: pytest.MonkeyPatch) -> None:
    class _Client:
        def __init__(self, *args: object, **kwargs: object) -> None:
            pass

        def __enter__(self) -> _Client:
            return self

        def __exit__(self, *args: object) -> None:
            return None

        def request(self, method: str, url: str, **kwargs: object) -> httpx.Response:
            return httpx.Response(
                302,
                headers={"location": "https://example.com/final"},
                request=httpx.Request("GET", url),
            )

    monkeypatch.setattr("vsh.http.fetch.httpx.Client", _Client)
    result = fetch_http(url="https://example.com/start", max_bytes=100, follow_redirects=False)
    assert result.status_code == 302


def test_fetch_redirect_without_location(monkeypatch: pytest.MonkeyPatch) -> None:
    class _Client:
        def __init__(self, *args: object, **kwargs: object) -> None:
            pass

        def __enter__(self) -> _Client:
            return self

        def __exit__(self, *args: object) -> None:
            return None

        def request(self, method: str, url: str, **kwargs: object) -> httpx.Response:
            return httpx.Response(302, headers={}, request=httpx.Request("GET", url))

    monkeypatch.setattr("vsh.http.fetch.httpx.Client", _Client)
    result = fetch_http(url="https://example.com/start", max_bytes=100)
    assert result.status_code == 302


def test_ssrf_private_ip_resolution(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("VSH_HTTP_ALLOWED_HOSTS", raising=False)

    def _fake_getaddrinfo(host: str, *args: object, **kwargs: object) -> list[tuple]:
        return [(socket.AF_INET, socket.SOCK_STREAM, 6, "", ("127.0.0.1", 0))]

    import socket

    monkeypatch.setattr(socket, "getaddrinfo", _fake_getaddrinfo)
    with pytest.raises(SsrfBlockedError):
        validate_outbound_url("https://resolver.test")


def test_snapshot_cache_fingerprint_oserror(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    cache = SnapshotCache()
    monkeypatch.setattr(Path, "stat", MagicMock(side_effect=OSError("nope")))
    assert cache.workspace_fingerprint(tmp_path) == 0


def test_snapshot_ignore_relative_error(tmp_path: Path) -> None:
    matcher = SnapshotIgnoreMatcher(workspace_root=tmp_path)
    assert matcher.is_ignored(Path("/outside/file"), is_dir=False) is False


def test_grep_read_scope_empty_children(tmp_path: Path) -> None:
    root = tmp_path / "workspace"
    (root / "empty").mkdir(parents=True)
    snapshot = snapshot_workspace(str(root), cwd=str(root))
    result = simulate_command(GrepCommand(pattern="x", path="empty", recursive=False), snapshot)
    assert result.decision != "reject"


def test_backup_file_helper(tmp_path: Path) -> None:
    from vsh.execute.rollback import RollbackSession, backup_file

    target = tmp_path / "f.txt"
    target.write_text("x\n", encoding="utf-8")
    session = RollbackSession(backup_root=tmp_path / "backup")
    backup_file(session, target)
    assert session.entries


def test_patch_create_via_dispatch(tmp_path: Path) -> None:
    ctx = ExecutionContext(workspace_root=str(tmp_path), cwd_logical=str(tmp_path))
    command = ApplyPatchCommand(
        path="created.txt",
        patch="\n===\ncontent\n",
        execution_reason="create via patch",
    )
    effects = apply_command(command, ctx)
    assert effects.creates == [str(tmp_path / "created.txt")]


def test_grep_scope_empty_directory(tmp_path: Path) -> None:
    root = tmp_path / "workspace"
    (root / "empty").mkdir(parents=True)
    snapshot = snapshot_workspace(str(root), cwd=str(root))
    result = simulate_command(GrepCommand(pattern="x", path="empty", recursive=False), snapshot)
    assert result.journal.content_reads


def test_snapshot_ignore_env_extra(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    monkeypatch.setenv("VSH_SNAPSHOT_IGNORE", "skipme.txt")
    (tmp_path / "skipme.txt").write_text("x", encoding="utf-8")
    snapshot = snapshot_workspace(str(tmp_path), cwd=str(tmp_path))
    assert str((tmp_path / "skipme.txt").resolve()) not in snapshot.nodes


def test_validation_hints_for_alias_fields() -> None:
    from pydantic import ValidationError

    from vsh.mcp.tools import _validation_error_hint

    directory_exc = ValidationError.from_exception_data(
        "test",
        cast(Any, [{"type": "string_type", "loc": ("directory",), "input": 1}]),
    )
    assert _validation_error_hint(directory_exc, tool_name="vsh_mkdir") == (
        "use params.path instead of params.directory"
    )
    grep_exc = ValidationError.from_exception_data(
        "test",
        cast(Any, [{"type": "string_type", "loc": ("root_directory",), "input": 1}]),
    )
    assert _validation_error_hint(grep_exc, tool_name="vsh_grep") == (
        "use params.path instead of params.root_directory"
    )
    echo_exc = ValidationError.from_exception_data(
        "test",
        cast(Any, [{"type": "string_type", "loc": ("file_path",), "input": 1}]),
    )
    assert _validation_error_hint(echo_exc, tool_name="vsh_echo") == (
        "use params.output_path (and params.text or params.content) for vsh_echo"
    )


def test_compile_grep_pattern_extended(tmp_path: Path) -> None:
    compiled = compile_grep_pattern(
        "a+", ignore_case=False, fixed_strings=False, extended_regexp=True
    )
    assert compiled is not None
    assert compiled.search("aaa") is not None


def test_snapshot_builder_oserror_iterdir(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    from vsh.snapshot import builder

    original_iterdir = Path.iterdir

    def _boom(self: Path):  # type: ignore[no-untyped-def]
        if self == tmp_path:
            raise OSError("denied")
        return original_iterdir(self)

    monkeypatch.setattr(Path, "iterdir", _boom)
    nodes = builder._build_nodes(tmp_path)  # noqa: SLF001
    assert str(tmp_path) in nodes


def test_apply_batch_transactional_rollback_after_rejected_step(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    result = tools.apply_batch(
        [
            {
                "tool_name": "vsh_mkdir",
                "params": {"path": "bench"},
                "execution_reason": "create",
            },
            {"tool_name": "vsh_mkdir", "params": {"path": "bench2"}},
        ],
        workspace_root=str(workspace),
        transactional=True,
    )
    assert result["status"] == "error"
    assert not (workspace / "bench").exists()


def test_ssrf_dns_edge_cases(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("VSH_HTTP_ALLOWED_HOSTS", raising=False)

    def _gaierror(*args: object, **kwargs: object) -> list[tuple]:
        raise socket.gaierror("no such host")

    monkeypatch.setattr(socket, "getaddrinfo", _gaierror)
    assert validate_outbound_url("https://unknown-host.test") == "https://unknown-host.test"

    def _bad_ip(*args: object, **kwargs: object) -> list[tuple]:
        return [(socket.AF_INET, socket.SOCK_STREAM, 6, "", ("not-an-ip", 0))]

    monkeypatch.setattr(socket, "getaddrinfo", _bad_ip)
    assert validate_outbound_url("https://weird.test") == "https://weird.test"

    with pytest.raises(SsrfBlockedError, match="missing a host"):
        validate_outbound_url("https:///no-host")

    from vsh.http.ssrf import resolve_allowed_host

    assert resolve_allowed_host("https://example.com") == "https://example.com"


def test_ssrf_blocks_literal_private_ip(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.delenv("VSH_HTTP_ALLOWED_HOSTS", raising=False)
    with pytest.raises(SsrfBlockedError):
        validate_outbound_url("https://127.0.0.1")


def test_grep_regex_timeout_can_be_disabled(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setenv("VSH_GREP_REGEX_TIMEOUT_SECS", "0")
    assert grep_regex_timeout_secs() == 0.0


def test_regex_timeout_noop_and_alarm(monkeypatch: pytest.MonkeyPatch) -> None:
    from vsh.limits import regex_timeout

    with regex_timeout(0):
        pass
    if not hasattr(signal, "SIGALRM"):
        return

    handlers: list[Callable[[int, object], None]] = []
    real_signal = signal.signal

    def _capture(signum: int, handler: Callable[[int, object], None]) -> object:
        handlers.append(handler)
        return real_signal(signum, handler)

    monkeypatch.setattr(signal, "signal", _capture)
    monkeypatch.setattr(signal, "setitimer", lambda *_args, **_kwargs: None)
    with pytest.raises(TimeoutError, match="regex execution timed out"), regex_timeout(0.001):
        assert handlers
        handlers[-1](signal.SIGALRM, None)


def test_grep_line_number_hits_max_matches(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr("vsh.execute.read_output.grep_max_matches", lambda: 1)
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "a.txt").write_text("hit\nhit\n", encoding="utf-8")
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))
    effects = apply_command(
        GrepCommand(pattern="hit", path="a.txt", line_number=True, recursive=False),
        ctx,
    )
    assert effects.stdout is not None
    assert len(effects.stdout.splitlines()) == 1


def test_grep_simulate_on_file_path(tmp_path: Path) -> None:
    root = tmp_path / "workspace"
    root.mkdir()
    (root / "only.txt").write_text("needle\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(root), cwd=str(root))
    result = simulate_command(
        GrepCommand(pattern="needle", path="only.txt", recursive=False),
        snapshot,
    )
    assert result.decision != "reject"


def test_snapshot_cache_max_age_invalid(monkeypatch: pytest.MonkeyPatch, tmp_path: Path) -> None:
    from vsh.snapshot import builder

    monkeypatch.setenv("VSH_SNAPSHOT_CACHE_MAX_AGE_SECS", "not-a-float")
    assert builder._cache_max_age_seconds() == 30.0  # noqa: SLF001


def test_snapshot_builder_skips_ignored_dir_on_stack(
    monkeypatch: pytest.MonkeyPatch, tmp_path: Path
) -> None:
    from vsh.snapshot import builder

    (tmp_path / "child").mkdir()
    calls = 0
    original = SnapshotIgnoreMatcher.is_ignored

    def _ignored_once(self: SnapshotIgnoreMatcher, path: Path, *, is_dir: bool) -> bool:
        nonlocal calls
        calls += 1
        if path.name == "child" and calls > 1:
            return True
        return original(self, path, is_dir=is_dir)

    monkeypatch.setattr(SnapshotIgnoreMatcher, "is_ignored", _ignored_once)
    nodes = builder._build_nodes(tmp_path)  # noqa: SLF001
    assert str(tmp_path / "child") not in nodes


def test_validation_error_hint_empty_loc() -> None:
    from pydantic import ValidationError

    from vsh.mcp.tools import _validation_error_hint

    exc = ValidationError.from_exception_data(
        "test",
        cast(Any, [{"type": "missing", "loc": (), "input": {}}]),
    )
    assert _validation_error_hint(exc, tool_name="vsh_mkdir") == (
        "check get_schema for required fields"
    )


def test_classify_generic_validation_error() -> None:
    from vsh.mcp.tools import _classify_apply_error, _compact_error

    code, hint = _classify_apply_error(None, tool_name="x", reason="bad field value")
    assert code == "validation_error"
    assert hint is None
    receipt = _compact_error(
        tool_name="x",
        snapshot_id=None,
        reason="bad field value",
        hint=None,
    )
    assert "hint" not in receipt


def test_apply_batch_transactional_flag(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    ok = tools.apply_batch(
        [
            {
                "tool_name": "vsh_mkdir",
                "params": {"path": "ok"},
                "execution_reason": "create",
            }
        ],
        workspace_root=str(workspace),
        transactional=True,
    )
    assert ok["status"] == "ok"
    assert (workspace / "ok").is_dir()
