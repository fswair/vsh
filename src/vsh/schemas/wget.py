from __future__ import annotations as _annotations

import shlex
from typing import ClassVar

from pydantic import Field

from .common import CommandKind, SideEffect, StructuredCommand

__all__ = ("WgetCommand",)


class WgetCommand(StructuredCommand):
    """Download a URL over HTTP(S) into the workspace."""

    _command_alias: ClassVar[str] = "wget"
    _positional_fields: ClassVar[tuple[str, ...]] = ("url",)

    kind: CommandKind = Field(
        default="write",
        description="Command category for HTTP downloads that write to the workspace.",
    )
    side_effects: list[SideEffect] = Field(
        default_factory=lambda: [
            SideEffect(kind="read", risks=["performs an outbound HTTP request"]),
            SideEffect(kind="write", risks=["writes the downloaded response body to a file"]),
        ],
        description="Declared side effects for this command.",
    )
    url: str = Field(description="HTTP or HTTPS URL to download.")
    output_path: str | None = Field(
        default=None,
        description="Optional workspace-relative output path (-O). Defaults to the URL basename.",
    )
    quiet: bool = Field(default=False, description="Suppress non-error output (-q).")
    follow_redirects: bool = Field(
        default=True,
        description="Follow HTTP redirects.",
    )
    max_bytes: int = Field(
        default=1_048_576,
        ge=1,
        description="Maximum response body size to download.",
    )

    def to_shell(self) -> str:
        tokens = [self._command_alias]
        if self.quiet:
            tokens.append("-q")
        if self.follow_redirects:
            tokens.append("-L")
        if self.output_path is not None:
            tokens.extend(["-O", shlex.quote(self.output_path)])
        tokens.append(shlex.quote(self.url))
        return " ".join(tokens)
