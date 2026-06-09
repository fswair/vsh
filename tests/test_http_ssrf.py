from __future__ import annotations as _annotations

import pytest

from vsh.http.ssrf import SsrfBlockedError, validate_outbound_url


def test_validate_outbound_url_blocks_localhost() -> None:
    with pytest.raises(SsrfBlockedError):
        validate_outbound_url("http://localhost/admin")


def test_validate_outbound_url_allows_public_host() -> None:
    assert validate_outbound_url("https://example.com/path") == "https://example.com/path"
