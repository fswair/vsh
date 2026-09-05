"""Credential-free end-to-end MCP protocol example over FastMCP's in-process transport."""

from __future__ import annotations

import asyncio
import json
from pathlib import Path
from tempfile import TemporaryDirectory

from fastmcp import Client

from vsh.mcp.codemode_server import create_codemode_server


async def run_workflow() -> dict[str, object]:
    with TemporaryDirectory(prefix="vsh-cookbook-mcp-") as directory:
        workspace = Path(directory)
        server = create_codemode_server()
        async with Client(server) as client:
            assert [tool.name for tool in await client.list_tools()] == ["vsh_run"]
            response = await client.call_tool(
                "vsh_run",
                {
                    "code": "vsh_write('/workspace/status.txt', 'ready\\n')\n'ready'",
                    "workspace_root": str(workspace),
                    "mode": "preview",
                    "detail": "full",
                },
            )
            preview = response.data
            assert isinstance(preview, dict)
            assert preview["decision"] == "auto_approved"
            assert preview["changes"] == [{"path": "status.txt", "kind": "create"}]
            assert preview["result_repr"] == "'ready'"
            assert preview["result_truncated"] is False
            assert not (workspace / "status.txt").exists()
            # This known fixture program is reviewed by the trusted host. An
            # arbitrary agent must not approve itself from its own result text.
            response = await client.call_tool(
                "vsh_run",
                {
                    "transaction": preview["transaction"],
                    "workspace_root": str(workspace),
                    "mode": "auto",
                },
            )
            committed = response.data
            assert isinstance(committed, dict)
            assert committed["transaction"] == preview["transaction"]
            assert committed["commit"]["committed"] is True
            assert (workspace / "status.txt").read_text() == "ready\n"
            return {"tool_count": 1, "state": committed["state"], "committed": True}


def main() -> None:
    print(json.dumps(asyncio.run(run_workflow()), indent=2))


if __name__ == "__main__":
    main()
