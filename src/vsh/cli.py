"""Dependency-free CLI composition over the native PyO3 runtime."""

from __future__ import annotations

import argparse
import base64
import json
import os
import sys
from collections.abc import Sequence
from pathlib import Path

from . import VshBashError, __version__


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="vsh", description="Run VSH's native Rust engine")
    parser.add_argument("--version", action="version", version=__version__)
    commands = parser.add_subparsers(dest="command", required=True)

    run = commands.add_parser("run", help="run one virtual filesystem transaction")
    source = run.add_mutually_exclusive_group(required=True)
    source.add_argument("--code", help="guest source text in the selected language")
    source.add_argument("--file", type=Path, help="read guest source from a UTF-8 file")
    source.add_argument("--transaction", help="promote an exact preview transaction")
    run.add_argument("--workspace", type=Path, default=Path.cwd())
    run.add_argument("--intent")
    run.add_argument("--language", choices=("monty", "bash"), default="monty")
    run.add_argument(
        "--enable-bash", action="store_true", help="host opt-in to the bundled bounded Bash worker"
    )
    run.add_argument("--bash-worker", type=Path, help="explicit trusted Bash worker executable")
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

    if arguments.enable_bash:
        os.environ["VSH_ENABLE_BASH"] = "1"
    if arguments.bash_worker is not None:
        os.environ["VSH_BASH_WORKER"] = str(arguments.bash_worker)

    code = arguments.code
    if arguments.file is not None:
        code = arguments.file.read_text(encoding="utf-8")
    try:
        payload = vsh_run(
            code,
            language=arguments.language,
            transaction=arguments.transaction,
            workspace_root=str(arguments.workspace),
            intent=arguments.intent,
            mode=arguments.mode,
            policy=arguments.policy,
            detail=arguments.detail,
        )
    except VshBashError as error:
        diagnostics = error.diagnostics
        limit = 64 * 1024
        failure = {
            "state": "failed",
            "language": "bash",
            "committable": False,
            "error": {"kind": diagnostics.kind, "message": str(error)},
            "exit_code": diagnostics.exit_code,
            "encoding": "base64",
            "stdout": base64.b64encode(diagnostics.stdout[:limit]).decode("ascii"),
            "stderr": base64.b64encode(diagnostics.stderr[:limit]).decode("ascii"),
            "output_truncated": len(diagnostics.stdout) > limit or len(diagnostics.stderr) > limit,
            "changes": [{"path": item.path, "kind": item.kind} for item in diagnostics.changes],
            "changes_complete": diagnostics.changes_complete,
            "denied_accesses": diagnostics.denied_accesses,
        }
        print(json.dumps(failure, ensure_ascii=False, separators=(",", ":")), file=sys.stderr)
        status = diagnostics.exit_code
        raise SystemExit(status if status is not None and 1 <= status <= 255 else 1) from None
    print(json.dumps(payload, ensure_ascii=False, separators=(",", ":")))
