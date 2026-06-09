from __future__ import annotations as _annotations

from dataclasses import dataclass
from urllib.parse import urljoin, urlparse

import httpx
from user_agent import generate_user_agent

from vsh.http.ssrf import validate_outbound_url

__all__ = (
    "HttpFetchResult",
    "build_request_headers",
    "fetch_http",
    "parse_curl_headers",
    "validate_http_url",
)

_ALLOWED_METHODS = frozenset({"GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"})


@dataclass(frozen=True, kw_only=True)
class HttpFetchResult:
    url: str
    status_code: int
    reason_phrase: str
    headers: dict[str, str]
    body: bytes
    stdout: str


def validate_http_url(url: str) -> str:
    """Return the normalized URL or raise ValueError for unsupported schemes."""
    parsed = urlparse(url.strip())
    if parsed.scheme not in {"http", "https"}:
        msg = f"only http and https URLs are supported: {url!r}"
        raise ValueError(msg)
    if not parsed.netloc:
        msg = f"URL is missing a host: {url!r}"
        raise ValueError(msg)
    return validate_outbound_url(url.strip())


def build_request_headers(headers: dict[str, str] | None) -> dict[str, str]:
    """Merge caller headers with a default User-Agent unless overridden."""
    merged: dict[str, str] = {"User-Agent": generate_user_agent()}
    for key, value in (headers or {}).items():
        if key.lower() == "user-agent":
            merged["User-Agent"] = value
        else:
            merged[key] = value
    return merged


def parse_curl_headers(header_lines: list[str]) -> dict[str, str]:
    headers: dict[str, str] = {}
    for line in header_lines:
        if ":" not in line:
            msg = f"invalid header format: {line!r}"
            raise ValueError(msg)
        name, value = line.split(":", 1)
        stripped_name = name.strip()
        if not stripped_name:
            msg = f"invalid header format: {line!r}"
            raise ValueError(msg)
        headers[stripped_name] = value.strip()
    return headers


def _normalize_method(method: str) -> str:
    normalized = method.strip().upper()
    if normalized not in _ALLOWED_METHODS:
        msg = f"unsupported HTTP method: {method!r}"
        raise ValueError(msg)
    return normalized


def _read_bounded_body(response: httpx.Response, *, max_bytes: int) -> bytes:
    body = response.content
    if len(body) > max_bytes:
        msg = f"response exceeds max_bytes ({max_bytes})"
        raise ValueError(msg)
    return body


def _format_response_headers(response: httpx.Response) -> str:
    version = response.http_version or "1.1"
    lines = [f"HTTP/{version} {response.status_code} {response.reason_phrase}"]
    for key, value in response.headers.multi_items():
        lines.append(f"{key}: {value}")
    return "\n".join(lines) + "\n\n"


def _decode_body(body: bytes) -> str:
    if not body:
        return ""
    try:
        return body.decode("utf-8")
    except UnicodeDecodeError:
        return body.decode("utf-8", errors="replace")


def fetch_http(
    *,
    url: str,
    method: str = "GET",
    headers: dict[str, str] | None = None,
    data: str | None = None,
    follow_redirects: bool = True,
    fail_on_error: bool = False,
    show_headers: bool = False,
    max_bytes: int,
    timeout_secs: float = 30.0,
) -> HttpFetchResult:
    """Perform an HTTP request with httpx and shape curl-like stdout."""
    validated_url = validate_http_url(url)
    normalized_method = _normalize_method(method)
    request_headers = build_request_headers(headers)
    content: str | bytes | None = data
    if content is not None and isinstance(content, str):
        content = content.encode("utf-8")

    with httpx.Client(follow_redirects=False, timeout=timeout_secs) as client:
        response = client.request(
            normalized_method,
            validated_url,
            headers=request_headers,
            content=content,
        )
        if follow_redirects:
            hops = 0
            while 300 <= response.status_code < 400 and hops < 5:
                location = response.headers.get("location")
                if location is None:
                    break
                next_url = validate_http_url(urljoin(str(response.url), location))
                response = client.request(
                    normalized_method,
                    next_url,
                    headers=request_headers,
                    content=content,
                )
                hops += 1

    if fail_on_error:
        response.raise_for_status()

    body = b"" if normalized_method == "HEAD" else _read_bounded_body(response, max_bytes=max_bytes)
    header_map = dict(response.headers.items())
    body_text = _decode_body(body)
    stdout = _format_response_headers(response) + body_text if show_headers else body_text

    return HttpFetchResult(
        url=validated_url,
        status_code=response.status_code,
        reason_phrase=response.reason_phrase,
        headers=header_map,
        body=body,
        stdout=stdout,
    )
