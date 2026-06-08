from __future__ import annotations as _annotations

from .fetch import (
    HttpFetchResult,
    build_request_headers,
    fetch_http,
    parse_curl_headers,
    validate_http_url,
)
from .wget import default_wget_output_name

__all__ = (
    "HttpFetchResult",
    "build_request_headers",
    "default_wget_output_name",
    "fetch_http",
    "parse_curl_headers",
    "validate_http_url",
)
