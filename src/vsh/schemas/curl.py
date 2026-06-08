from __future__ import annotations as _annotations

import shlex
from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand

__all__ = ("CurlCommand",)


class CurlCommand(StructuredCommand):
    """Fetch a URL over HTTP(S) and print the response or save it to a file."""

    _command_alias: ClassVar[str] = "curl"
    _positional_fields: ClassVar[tuple[str, ...]] = ("url",)

    kind: CommandKind = Field(
        default="read",
        description="Command category for HTTP reads and optional file writes.",
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [
            SideEffect(kind="read", risks=["performs an outbound HTTP request"]),
            SideEffect(kind="write", risks=["may write the response body to a file"]),
        ],
        description="Declared side effects for this command.",
    )
    url: str = Field(description="HTTP or HTTPS URL to request.")
    method: str = Field(default="GET", description="HTTP method to use.")
    headers: list[str] = Field(
        default_factory=list,
        description='Request headers in curl form, for example "Accept: application/json".',
    )
    data: str | None = Field(
        default=None,
        description="Optional request body for POST, PUT, or PATCH requests.",
    )
    output_path: str | None = Field(
        default=None,
        description="Optional workspace-relative path to write the response body (-o).",
    )
    silent: bool = Field(default=False, description="Suppress progress output (-s).")
    show_headers: bool = Field(
        default=False,
        description="Include response headers in stdout (-i).",
    )
    fail_on_error: bool = Field(
        default=False,
        description="Treat HTTP status codes >= 400 as errors (-f).",
    )
    follow_redirects: bool = Field(
        default=True,
        description="Follow HTTP redirects (-L).",
    )
    max_bytes: int = Field(
        default=1_048_576,
        ge=1,
        description="Maximum response body size to download.",
    )

    def to_shell(self) -> str:
        tokens = [self._command_alias]
        if self.silent:
            tokens.append("-s")
        if self.show_headers:
            tokens.append("-i")
        if self.fail_on_error:
            tokens.append("-f")
        if self.follow_redirects:
            tokens.append("-L")
        if self.method.upper() != "GET":
            tokens.extend(["-X", shlex.quote(self.method)])
        for header in self.headers:
            tokens.extend(["-H", shlex.quote(header)])
        if self.data is not None:
            tokens.extend(["-d", shlex.quote(self.data)])
        if self.output_path is not None:
            tokens.extend(["-o", shlex.quote(self.output_path)])
        tokens.append(shlex.quote(self.url))
        return " ".join(tokens)
