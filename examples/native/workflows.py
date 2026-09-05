"""Executable native SDK cookbook; all writes belong to disposable fixtures.

Run from a checkout with its release extension and matching worker installed:
    uv run --no-sync python examples/native/workflows.py
"""

from __future__ import annotations

import json
import time
from pathlib import Path
from tempfile import TemporaryDirectory

from vsh import Receipt, ReceiptDetail, Runtime, VshStaleError, VshStateError

STAGED_RELEASE = (
    Path(__file__).resolve().parents[2] / "crates/vbash/examples/staged_release.monty"
).read_text(encoding="utf-8")

MIGRATION = """
limit = 20
files = vsh_glob('**/*.toml', path='/workspace/services', max_results=limit + 1)
assert 0 < len(files) <= limit, 'Split this migration into explicitly reviewed batches'
review = []
for file in files:
    before = vsh_read(file)
    assert before.count('timeout = 5') == 1, 'Unexpected config; do not guess'
    assert vsh_patch(file, 'timeout = 5', 'timeout = 15') == 1
    review.append({'path': str(file), 'before': before, 'after': vsh_read(file)})
review
"""

DERIVED_OUTPUT = """
source = vsh_read('/workspace/input.txt')
vsh_write('/workspace/output.txt', source.upper())
{'before': source, 'after': vsh_read('/workspace/output.txt')}
"""


def now_ms() -> int:
    return time.time_ns() // 1_000_000


def summary(receipt: Receipt) -> dict[str, object]:
    return {
        "state": receipt.state,
        "decision": receipt.decision,
        "changed_paths": receipt.changed_paths,
        "committed": receipt.committed,
    }


def migrate_configs() -> dict[str, object]:
    """Review bounded before/after content, then promote that exact preview."""
    with TemporaryDirectory(prefix="vsh-cookbook-migration-") as directory:
        workspace = Path(directory)
        names = ("billing", "search")
        for name in names:
            service = workspace / "services" / name
            service.mkdir(parents=True)
            (service / "service.toml").write_text("timeout = 5\n", encoding="utf-8")
        runtime = Runtime.open(workspace)
        preview = runtime.preview(
            MIGRATION, intent="Raise two fixture timeouts to 15 seconds", detail=ReceiptDetail.FULL
        )
        assert preview.decision == "auto_approved" and not preview.committed
        assert preview.changes == [(f"services/{name}/service.toml", "modify") for name in names]
        expected_review = [
            {
                "path": f"/workspace/services/{name}/service.toml",
                "before": "timeout = 5\n",
                "after": "timeout = 15\n",
            }
            for name in names
        ]
        # Trusted fixture review, not a model approving its own arbitrary output.
        assert preview.result == expected_review
        assert all(
            (workspace / "services" / name / "service.toml").read_text() == "timeout = 5\n"
            for name in names
        )
        committed = runtime.commit(preview.transaction, now_ms())
        assert committed.transaction == preview.transaction and committed.committed
        assert all(
            (workspace / "services" / name / "service.toml").read_text() == "timeout = 15\n"
            for name in names
        )
        return summary(committed)


def stage_release() -> dict[str, object]:
    """Compose copy, patch, move, pathlib and generation in one active overlay."""
    with TemporaryDirectory(prefix="vsh-cookbook-release-") as directory:
        workspace = Path(directory)
        (workspace / "templates").mkdir()
        (workspace / "templates/service.toml").write_text('channel = "dev"\n', encoding="utf-8")
        runtime = Runtime.open(workspace)
        preview = runtime.preview(STAGED_RELEASE, detail=ReceiptDetail.FULL)
        assert preview.decision == "pending_approval" and "rename" in preview.risk_flags
        assert preview.result == {"config": 'channel = "stable"\n', "files": 2}
        assert preview.changes == [
            ("release", "create"),
            ("release/README.txt", "create"),
            ("release/app.toml", "create"),
        ]
        assert not (workspace / "release").exists()
        # A second request starts a fresh snapshot, not the prior overlay.
        probe = runtime.preview("from pathlib import Path\nPath('/workspace/release').exists()")
        assert probe.result is False
        assert runtime.discard_preview(probe.transaction)
        # The semantic rename escalates even though its intermediate path is
        # absent from the final diff. The trusted fixture review above is exact.
        issued = now_ms()
        runtime.approve(preview.transaction, "fixture-reviewer", issued, issued + 30_000)
        committed = runtime.commit(preview.transaction, now_ms())
        assert committed.committed
        assert (workspace / "release/app.toml").read_text() == 'channel = "stable"\n'
        assert (workspace / "release/README.txt").read_text() == "channel=stable\n"
        assert not (workspace / "release/service.toml").exists()
        return summary(committed)


def approve_after_restart() -> dict[str, object]:
    """Strict pending artifacts survive restart; approval stays host-owned."""
    with TemporaryDirectory(prefix="vsh-cookbook-review-") as directory:
        workspace = Path(directory)
        (workspace / "input.txt").write_text("review me\n", encoding="utf-8")
        runtime = Runtime.open(workspace, policy="strict")
        preview = runtime.preview(DERIVED_OUTPUT, detail=ReceiptDetail.FULL)
        assert preview.state == "pending_approval"
        assert preview.changes == [("output.txt", "create")]
        assert preview.result == {"before": "review me\n", "after": "REVIEW ME\n"}
        try:
            runtime.commit(preview.transaction, now_ms())
        except VshStateError:
            pass
        else:
            raise AssertionError("Strict mutation committed without an approval")
        del runtime
        reopened = Runtime.open(workspace, policy="strict")
        issued = now_ms()
        # Only a trusted host may turn an authenticated review into this call.
        # A principal string by itself is not authentication.
        assert (
            reopened.approve(preview.transaction, "fixture-reviewer", issued, issued + 30_000)
            == "approved"
        )
        committed = reopened.commit(preview.transaction, now_ms())
        assert committed.committed and committed.transaction == preview.transaction
        assert (workspace / "output.txt").read_text() == "REVIEW ME\n"
        return summary(committed)


def reject_stale_input() -> dict[str, object]:
    """Preserve an external edit and leave the proposed output absent."""
    with TemporaryDirectory(prefix="vsh-cookbook-stale-") as directory:
        workspace = Path(directory)
        source = workspace / "input.txt"
        source.write_text("old\n", encoding="utf-8")
        runtime = Runtime.open(workspace)
        preview = runtime.preview(DERIVED_OUTPUT)
        source.write_text("an external editor changed this\n", encoding="utf-8")
        try:
            runtime.commit(preview.transaction, now_ms())
        except VshStaleError:
            pass
        else:
            raise AssertionError("Stale preview was committed")
        assert source.read_text() == "an external editor changed this\n"
        assert not (workspace / "output.txt").exists()
        return {"state": "stale", "external_edit_preserved": True, "committed": False}


def main() -> None:
    outcomes = {
        "migration": migrate_configs(),
        "release": stage_release(),
        "approval_after_restart": approve_after_restart(),
        "stale_input": reject_stale_input(),
    }
    print(json.dumps(outcomes, indent=2))


if __name__ == "__main__":
    main()
