"""Preview a filesystem change without mutating the host workspace."""

from __future__ import annotations

import json
from pathlib import Path
from tempfile import TemporaryDirectory

from vsh import ReceiptDetail, Runtime


def run() -> dict[str, object]:
    with TemporaryDirectory(prefix="vsh-preview-") as directory:
        workspace = Path(directory)
        (workspace / "input.txt").write_text("hello\n", encoding="utf-8")

        runtime = Runtime.open(workspace)
        receipt = runtime.preview(
            """
from pathlib import Path
source = Path('/workspace/input.txt').read_text()
Path('/workspace/output.txt').write_text(source.upper())
{'bytes': len(source), 'output': source.upper()}
""",
            intent="Preview an uppercase derivative",
            detail=ReceiptDetail.FULL,
        )

        assert receipt.decision == "auto_approved"
        assert receipt.changes == [("output.txt", "create")]
        assert receipt.result == {"bytes": 6, "output": "HELLO\n"}
        assert not (workspace / "output.txt").exists()
        assert runtime.discard_preview(receipt.transaction)
        return {
            "decision": receipt.decision,
            "changes": receipt.changes,
            "host_mutated": False,
        }


if __name__ == "__main__":
    print(json.dumps(run(), indent=2))
