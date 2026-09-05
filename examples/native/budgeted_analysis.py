"""Run read-only analysis with explicit resource and output limits."""

from __future__ import annotations

import json
from pathlib import Path
from tempfile import TemporaryDirectory

from vsh import ExecutionBudget, ReceiptDetail, Runtime


def run() -> dict[str, object]:
    with TemporaryDirectory(prefix="vsh-analysis-") as directory:
        workspace = Path(directory)
        (workspace / "service.toml").write_text("timeout = 15\n", encoding="utf-8")
        runtime = Runtime.open(workspace)
        receipt = runtime.preview(
            """
files = vsh_glob('*.toml', path='/workspace', max_results=11)
{'count': len(files), 'files': [str(path) for path in files]}
""",
            detail=ReceiptDetail.FULL,
            budget=ExecutionBudget(
                max_duration_ms=250,
                max_os_calls=100,
                max_read_bytes=1024 * 1024,
                max_output_bytes=16 * 1024,
                max_result_bytes=16 * 1024,
            ),
        )

        assert receipt.changed_paths == 0
        assert receipt.result == {"count": 1, "files": ["/workspace/service.toml"]}
        assert runtime.discard_preview(receipt.transaction)
        return {"result": receipt.result, "os_calls": receipt.os_calls}


if __name__ == "__main__":
    print(json.dumps(run(), indent=2))
