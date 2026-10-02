"""Disposable Bash preview, exact promotion and deterministic capability review."""

import asyncio
from pathlib import Path
from tempfile import TemporaryDirectory

from vsh import BashConfig, BashResult, HookDecision, Language, RequestEvent, Runtime
from vsh.pydantic_ai import VshCapability

with TemporaryDirectory(prefix="vsh-bash-example-") as temporary:
    workspace = Path(temporary)
    (workspace / "input.bin").write_bytes(b"\xff\x00verified\n")
    runtime = Runtime.open(workspace, bash=BashConfig())
    preview = runtime.preview("cat input.bin > report.bin; cat report.bin", language=Language.BASH)
    assert isinstance(preview.result, BashResult)
    assert preview.result.stdout == b"\xff\x00verified\n"
    assert not (workspace / "report.bin").exists()
    committed = runtime.commit(preview.transaction, 0)
    assert committed.state == "committed"
    assert (workspace / "report.bin").read_bytes() == b"\xff\x00verified\n"

    def review(event: RequestEvent) -> HookDecision:
        if event.evidence_complete and all(
            change.path == "reviewed.txt" for change in event.canonical_diff
        ):
            return HookDecision.approve("exact bounded fixture output is expected")
        return HookDecision.review("confirm the unexpected paths before applying changes")

    capability = VshCapability(workspace, bash=BashConfig(), policy="strict", hook_handler=review)
    result = asyncio.run(
        capability.vsh_run(
            "printf reviewed > reviewed.txt", "write one fixture report", language="bash"
        )
    )
    assert result.state == "committed"
    print(result)
