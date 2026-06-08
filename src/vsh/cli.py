from __future__ import annotations as _annotations

import json
from typing import Any

import click

from .registry import get_schema, search, search_names


@click.group()
def main() -> None:
    """vsh command registry tools."""


@main.command("search")
@click.argument("query")
def search_command(query: str) -> None:
    """Search command specs."""
    click.echo(json.dumps([spec.model_dump() for spec in search(query)], indent=2, sort_keys=True))


@main.command("schema")
@click.argument("name")
def schema_command(name: str) -> None:
    """Print a command JSON schema."""
    schema: dict[str, Any] = get_schema(name)
    click.echo(json.dumps(schema, indent=2, sort_keys=True))


@main.command("names")
@click.argument("query")
def names_command(query: str) -> None:
    """Search command names."""
    click.echo(json.dumps(search_names(query), indent=2, sort_keys=True))


@main.command("serve")
def serve_command() -> None:
    """Run the default FastMCP server."""
    from .mcp.server import mcp

    mcp.run()


@main.command("serve-codemode")
@click.option(
    "--instructions",
    "-i",
    default=None,
    help="Extra MCP server instructions appended to the CodeMode defaults.",
)
@click.option(
    "--instructions-file",
    "-f",
    type=click.Path(exists=True, dir_okay=False, readable=True, path_type=str),
    default=None,
    help="Read extra MCP instructions from a UTF-8 text file.",
)
def serve_codemode_command(instructions: str | None, instructions_file: str | None) -> None:
    """Run the CodeMode-style FastMCP server."""
    from .mcp.codemode_server import run_codemode_server

    run_codemode_server(inline=instructions, instructions_file=instructions_file)
