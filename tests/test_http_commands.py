from __future__ import annotations as _annotations

from pathlib import Path
from typing import cast

import httpx
import pytest

from vsh.execute.dispatch import ExecutionContext, apply_command
from vsh.execute.realfs import execute_approved
from vsh.http import default_wget_output_name, fetch_http, parse_curl_headers, validate_http_url
from vsh.plans.approval import approve_plan
from vsh.schemas import CurlCommand, WgetCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


class _FakeResponse:
    def __init__(self, *, body: bytes = b"hello", status_code: int = 200) -> None:
        self.status_code = status_code
        self.reason_phrase = "OK"
        self.http_version = "1.1"
        self.headers = httpx.Headers({"content-type": "text/plain"})
        self.content = body

    def raise_for_status(self) -> None:
        if self.status_code >= 400:
            request = httpx.Request("GET", "https://example.com")
            response = httpx.Response(self.status_code, request=request)
            raise httpx.HTTPStatusError("error", request=request, response=response)


class _FakeClient:
    last_method: str | None = None
    last_kwargs: dict[str, object] | None = None

    def __init__(self, *args: object, **kwargs: object) -> None:
        _ = (args, kwargs)

    def __enter__(self) -> _FakeClient:
        return self

    def __exit__(self, *args: object) -> None:
        _ = args

    def request(self, method: str, url: str, **kwargs: object) -> _FakeResponse:
        _FakeClient.last_method = method
        _FakeClient.last_kwargs = kwargs
        return _FakeResponse()


def _last_request_headers() -> dict[str, str]:
    assert _FakeClient.last_kwargs is not None
    headers = _FakeClient.last_kwargs.get("headers")
    assert isinstance(headers, dict)
    return cast(dict[str, str], headers)


@pytest.fixture
def fake_httpx_client(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(httpx, "Client", _FakeClient)


def test_validate_http_url_accepts_http_and_https() -> None:
    assert validate_http_url("https://example.com/path") == "https://example.com/path"


def test_validate_http_url_rejects_non_http_schemes() -> None:
    with pytest.raises(ValueError, match="only http and https"):
        validate_http_url("ftp://example.com/file")


def test_validate_http_url_requires_host() -> None:
    with pytest.raises(ValueError, match="missing a host"):
        validate_http_url("https://")


def test_parse_curl_headers_rejects_invalid_lines() -> None:
    with pytest.raises(ValueError, match="invalid header format"):
        parse_curl_headers(["not-a-header"])
    with pytest.raises(ValueError, match="invalid header format"):
        parse_curl_headers([": missing-name"])


def test_parse_curl_headers_parses_name_value_pairs() -> None:
    assert parse_curl_headers(["Accept: application/json"]) == {"Accept": "application/json"}


def test_build_request_headers_adds_default_user_agent(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from vsh.http import build_request_headers

    monkeypatch.setattr("vsh.http.fetch.generate_user_agent", lambda: "vsh-test-agent/1.0")
    assert build_request_headers(None) == {"User-Agent": "vsh-test-agent/1.0"}
    assert build_request_headers({"Accept": "application/json"}) == {
        "User-Agent": "vsh-test-agent/1.0",
        "Accept": "application/json",
    }


def test_build_request_headers_allows_user_agent_override(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from vsh.http import build_request_headers

    monkeypatch.setattr("vsh.http.fetch.generate_user_agent", lambda: "vsh-test-agent/1.0")
    assert build_request_headers({"user-agent": "custom-agent/9"}) == {
        "User-Agent": "custom-agent/9",
    }


def test_fetch_http_sets_default_user_agent(
    fake_httpx_client: None,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr("vsh.http.fetch.generate_user_agent", lambda: "vsh-test-agent/1.0")
    fetch_http(url="https://example.com", max_bytes=1024)
    assert _last_request_headers()["User-Agent"] == "vsh-test-agent/1.0"


def test_fetch_http_respects_user_agent_override(
    fake_httpx_client: None,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr("vsh.http.fetch.generate_user_agent", lambda: "vsh-test-agent/1.0")
    fetch_http(
        url="https://example.com",
        headers={"User-Agent": "custom-agent/9"},
        max_bytes=1024,
    )
    assert _last_request_headers()["User-Agent"] == "custom-agent/9"


def test_fetch_http_supports_headers_show_headers_and_head(
    fake_httpx_client: None,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr("vsh.http.fetch.generate_user_agent", lambda: "vsh-test-agent/1.0")
    result = fetch_http(
        url="https://example.com",
        method="HEAD",
        headers={"X-Test": "1"},
        data="payload",
        show_headers=True,
        max_bytes=1024,
    )
    assert result.stdout.startswith("HTTP/1.1 200")
    assert result.body == b""
    assert _FakeClient.last_method == "HEAD"
    assert _FakeClient.last_kwargs is not None
    assert _FakeClient.last_kwargs["content"] == b"payload"
    headers = _last_request_headers()
    assert headers["User-Agent"] == "vsh-test-agent/1.0"
    assert headers["X-Test"] == "1"


def test_fetch_http_rejects_unsupported_method(fake_httpx_client: None) -> None:
    with pytest.raises(ValueError, match="unsupported HTTP method"):
        fetch_http(url="https://example.com", method="TRACE", max_bytes=1024)


def test_fetch_http_enforces_max_bytes(
    fake_httpx_client: None, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        _FakeClient,
        "request",
        lambda self, method, url, **kwargs: _FakeResponse(body=b"x" * 32),
    )
    with pytest.raises(ValueError, match="exceeds max_bytes"):
        fetch_http(url="https://example.com", max_bytes=8)


def test_fetch_http_fail_on_error_raises(
    fake_httpx_client: None, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setattr(
        _FakeClient,
        "request",
        lambda self, method, url, **kwargs: _FakeResponse(status_code=404),
    )
    with pytest.raises(httpx.HTTPStatusError):
        fetch_http(url="https://example.com", fail_on_error=True, max_bytes=1024)


def test_fetch_http_decodes_binary_with_replacement(
    fake_httpx_client: None,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(
        _FakeClient,
        "request",
        lambda self, method, url, **kwargs: _FakeResponse(body=b"\xff\xfe"),
    )
    result = fetch_http(url="https://example.com", max_bytes=1024)
    assert "\ufffd" in result.stdout


def test_curl_and_wget_shell_previews() -> None:
    assert (
        repr(
            CurlCommand(
                url="https://example.com",
                method="POST",
                headers=["Accept: application/json"],
                data="{}",
                output_path="out.txt",
                show_headers=True,
                fail_on_error=True,
            )
        )
        == "curl -i -f -L -X POST -H 'Accept: application/json' -d '{}' -o out.txt https://example.com"
    )
    assert (
        repr(
            WgetCommand(url="https://example.com/readme.txt", output_path="readme.txt", quiet=True)
        )
        == "wget -q -L -O readme.txt https://example.com/readme.txt"
    )
    assert repr(CurlCommand(url="https://example.com", silent=True, follow_redirects=False)) == (
        "curl -s https://example.com"
    )
    assert repr(WgetCommand(url="https://example.com", follow_redirects=False)) == (
        "wget https://example.com"
    )


def test_default_wget_output_name_uses_basename_or_index_html() -> None:
    assert default_wget_output_name("https://example.com/docs/readme.txt") == "readme.txt"
    assert default_wget_output_name("https://example.com/") == "index.html"


def test_simulate_curl_stdout_only_is_approved(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(CurlCommand(url="https://example.com"), snapshot)
    assert result.decision == "approve"
    assert result.requires_manual_approval is True
    assert result.predicted_effects.reads == ["https://example.com"]


def test_simulate_curl_rejects_invalid_url(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(CurlCommand(url="file:///etc/passwd"), snapshot)
    assert result.decision == "reject"


def test_simulate_curl_with_output_predicts_file_update(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "page.html").write_text("old\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(
        CurlCommand(url="https://example.com", output_path="page.html"),
        snapshot,
    )
    target = str((workspace / "page.html").resolve())
    assert target in result.predicted_effects.updates


def test_simulate_curl_with_output_predicts_file_create(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(
        CurlCommand(url="https://example.com", output_path="page.html"),
        snapshot,
    )
    target = str((workspace / "page.html").resolve())
    assert result.decision == "approve_with_warning"
    assert target in result.predicted_effects.creates


def test_simulate_curl_rejects_workspace_escape(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(
        CurlCommand(url="https://example.com", output_path="../outside.txt"),
        snapshot,
    )
    assert result.decision == "reject"
    assert result.reason is not None
    assert "escapes workspace" in result.reason


def test_simulate_curl_rejects_protected_output_path(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(
        CurlCommand(url="https://example.com", output_path=".env"),
        snapshot,
    )
    assert result.decision == "reject"


def test_simulate_wget_rejects_invalid_url_and_escape(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    invalid = simulate_command(WgetCommand(url="not-a-url"), snapshot)
    escaped = simulate_command(
        WgetCommand(url="https://example.com", output_path="../outside.txt"),
        snapshot,
    )
    assert invalid.decision == "reject"
    assert escaped.decision == "reject"


def test_simulate_wget_rejects_protected_output_path(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(
        WgetCommand(url="https://example.com", output_path=".env"),
        snapshot,
    )
    assert result.decision == "reject"


def test_simulate_wget_predicts_download_path(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(WgetCommand(url="https://example.com/readme.txt"), snapshot)
    target = str((workspace / "readme.txt").resolve())
    assert result.decision == "approve_with_warning"
    assert target in result.predicted_effects.creates


def test_simulate_wget_predicts_file_update(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    (workspace / "readme.txt").write_text("old\n", encoding="utf-8")
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(
        WgetCommand(url="https://example.com/readme.txt", output_path="readme.txt"),
        snapshot,
    )
    target = str((workspace / "readme.txt").resolve())
    assert target in result.predicted_effects.updates


def test_apply_curl_writes_stdout(fake_httpx_client: None, tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))
    effects = apply_command(CurlCommand(url="https://example.com"), ctx)
    assert effects.stdout == "hello"
    assert effects.reads == ["https://example.com"]


def test_apply_curl_updates_existing_output_file(fake_httpx_client: None, tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "page.html"
    target.write_text("old\n", encoding="utf-8")
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))
    effects = apply_command(
        CurlCommand(url="https://example.com", output_path="page.html"),
        ctx,
    )
    assert target.read_bytes() == b"hello"
    assert str(target.resolve()) in effects.updates


def test_apply_curl_writes_output_file(fake_httpx_client: None, tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))
    target = workspace / "page.html"
    effects = apply_command(
        CurlCommand(url="https://example.com", output_path="page.html"),
        ctx,
    )
    assert target.read_bytes() == b"hello"
    assert str(target.resolve()) in effects.creates


def test_apply_wget_updates_existing_file_and_honors_quiet(
    fake_httpx_client: None,
    tmp_path: Path,
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "readme.txt"
    target.write_text("old\n", encoding="utf-8")
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))
    effects = apply_command(
        WgetCommand(url="https://example.com/readme.txt", quiet=True),
        ctx,
    )
    assert str(target.resolve()) in effects.updates
    assert effects.stdout == ""


def test_apply_wget_downloads_to_default_name(fake_httpx_client: None, tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    ctx = ExecutionContext(workspace_root=str(workspace), cwd_logical=str(workspace))
    effects = apply_command(WgetCommand(url="https://example.com/readme.txt"), ctx)
    target = workspace / "readme.txt"
    assert target.read_bytes() == b"hello"
    assert str(target.resolve()) in effects.creates


def test_execute_approved_curl_plan(fake_httpx_client: None, tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(CurlCommand(url="https://example.com"), snapshot)
    token = approve_plan(result.plan_id)
    execution = execute_approved(token.token)
    assert execution.applied is True
    assert execution.actual_effects is not None
    assert execution.actual_effects.stdout == "hello"
