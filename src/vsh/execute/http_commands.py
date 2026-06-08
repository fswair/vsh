from __future__ import annotations as _annotations

from pathlib import Path

from vsh.effects import ActualEffects
from vsh.http import default_wget_output_name, fetch_http, parse_curl_headers
from vsh.schemas import CurlCommand, WgetCommand

from .dispatch import ExecutionContext

__all__ = ("apply_curl_command", "apply_wget_command")


def apply_curl_command(command: CurlCommand, ctx: ExecutionContext) -> ActualEffects:
    result = fetch_http(
        url=command.url,
        method=command.method,
        headers=parse_curl_headers(command.headers),
        data=command.data,
        follow_redirects=command.follow_redirects,
        fail_on_error=command.fail_on_error,
        show_headers=command.show_headers,
        max_bytes=command.max_bytes,
    )
    if command.output_path is None:
        return ActualEffects(
            reads=[result.url],
            cwd_after=ctx.cwd_logical,
            stdout=result.stdout,
        )

    target = ctx.resolve_within_workspace(command.output_path)
    path = Path(target)
    path.parent.mkdir(parents=True, exist_ok=True)
    existed_before = path.exists()
    path.write_bytes(result.body)
    if existed_before:
        return ActualEffects(
            reads=[result.url],
            updates=[target],
            cwd_after=ctx.cwd_logical,
            stdout="",
        )
    return ActualEffects(
        reads=[result.url],
        creates=[target],
        cwd_after=ctx.cwd_logical,
        stdout="",
    )


def apply_wget_command(command: WgetCommand, ctx: ExecutionContext) -> ActualEffects:
    relative_output = command.output_path or default_wget_output_name(command.url)
    target = ctx.resolve_within_workspace(relative_output)
    result = fetch_http(
        url=command.url,
        method="GET",
        follow_redirects=command.follow_redirects,
        fail_on_error=True,
        show_headers=False,
        max_bytes=command.max_bytes,
    )
    path = Path(target)
    path.parent.mkdir(parents=True, exist_ok=True)
    existed_before = path.exists()
    path.write_bytes(result.body)
    stdout = "" if command.quiet else f"{target}\n"
    if existed_before:
        return ActualEffects(
            reads=[result.url],
            updates=[target],
            cwd_after=ctx.cwd_logical,
            stdout=stdout,
        )
    return ActualEffects(
        reads=[result.url],
        creates=[target],
        cwd_after=ctx.cwd_logical,
        stdout=stdout,
    )
