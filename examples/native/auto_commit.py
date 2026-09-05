"""Run a trusted bounded transformation and commit it in one native request."""

from __future__ import annotations

import json
from pathlib import Path
from tempfile import TemporaryDirectory

from vsh import ReceiptDetail, RunMode, RunRequest, Runtime


def run() -> dict[str, object]:
    with TemporaryDirectory(prefix="vsh-auto-") as directory:
        workspace = Path(directory)
        runtime = Runtime.open(workspace)
        receipt = runtime.run(
            RunRequest(
                "vsh_write('/workspace/status.txt', 'ready\\n')\n'ready'",
                intent="Create a reviewed fixture status file",
                mode=RunMode.AUTO,
                detail=ReceiptDetail.FULL,
            )
        )

        assert receipt.state == "committed"
        assert receipt.committed
        assert receipt.result == "ready"
        assert (workspace / "status.txt").read_text(encoding="utf-8") == "ready\n"
        return {
            "state": receipt.state,
            "transaction": receipt.transaction,
            "committed": receipt.committed,
        }


if __name__ == "__main__":
    print(json.dumps(run(), indent=2))
