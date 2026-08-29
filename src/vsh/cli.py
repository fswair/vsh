"""Dependency-free CLI composition over the native PyO3 runtime."""

from __future__ import annotations

import argparse
import json
from collections.abc import Sequence
from pathlib import Path

from . import __version__


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="vsh", description="Run VSH's native Rust engine")
    parser.add_argument("--version", action="version", version=__version__)
    commands = parser.add_subparsers(dest="command", required=True)

    run = commands.add_parser("run", help="run one Monty transaction")
    source = run.add_mutually_exclusive_group(required=True)
    source.add_argument("--code", help="Monty source text")
    source.add_argument("--file", type=Path, help="read Monty source from a UTF-8 file")
    source.add_argument("--transaction", help="promote an exact preview transaction")
    run.add_argument("--workspace", type=Path, default=Path.cwd())
    run.add_argument("--intent")
    run.add_argument("--mode", choices=("preview", "auto"), default="preview")
    run.add_argument("--policy", choices=("balanced", "strict", "paranoid"), default="balanced")
    run.add_argument("--detail", choices=("compact", "full"), default="compact")

    commands.add_parser("serve", help="serve the single-tool MCP surface over stdio")
    return parser


def main(argv: Sequence[str] | None = None) -> None:
    """Run the VSH CLI."""
    arguments = _parser().parse_args(argv)
    if arguments.command == "serve":
        from .mcp.server import mcp

        mcp.run()
        return

    from .mcp.native_tools import vsh_run

    code = arguments.code
    if arguments.file is not None:
        code = arguments.file.read_text(encoding="utf-8")
    payload = vsh_run(
        code,
        transaction=arguments.transaction,
        workspace_root=str(arguments.workspace),
        intent=arguments.intent,
        mode=arguments.mode,
        policy=arguments.policy,
        detail=arguments.detail,
    )
    print(json.dumps(payload, ensure_ascii=False, separators=(",", ":")))
