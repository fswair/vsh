"""Approve and commit the exact durable transaction produced under strict policy."""

from __future__ import annotations

import json
import time
from pathlib import Path
from tempfile import TemporaryDirectory

from vsh import ReceiptDetail, Runtime, VshStateError


def now_ms() -> int:
    return time.time_ns() // 1_000_000


def run() -> dict[str, object]:
    with TemporaryDirectory(prefix="vsh-review-") as directory:
        workspace = Path(directory)
        runtime = Runtime.open(workspace, policy="strict")
        preview = runtime.preview(
            "vsh_write('/workspace/reviewed.txt', 'approved\\n')",
            intent="Create a fixture file after independent review",
            detail=ReceiptDetail.FULL,
        )

        assert preview.decision == "pending_approval"
        assert not (workspace / "reviewed.txt").exists()
        try:
            runtime.commit(preview.transaction, now_ms())
        except VshStateError:
            pass
        else:
            raise AssertionError("strict transaction committed without approval")

        issued = now_ms()
        state = runtime.approve(
            preview.transaction,
            "authenticated-fixture-reviewer",
            issued,
            issued + 30_000,
        )
        committed = runtime.commit(preview.transaction, now_ms())

        assert state == "approved"
        assert committed.committed
        assert (workspace / "reviewed.txt").read_text(encoding="utf-8") == "approved\n"
        return {"review_state": state, "commit_state": committed.state}


if __name__ == "__main__":
    print(json.dumps(run(), indent=2))
