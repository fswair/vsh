from __future__ import annotations

import asyncio
import base64
import os
import re
import runpy
import subprocess
import sys
import threading
import time
from collections.abc import Callable
from pathlib import Path
from typing import Literal, cast

import pytest

from vsh import (
    BashConfig,
    BashLimits,
    BashResult,
    ExecutionBudget,
    HookDecision,
    HookedRuntime,
    HookScope,
    Language,
    Receipt,
    ReceiptDetail,
    RequestEvent,
    RunMode,
    RunRequest,
    Runtime,
    VshBashError,
    VshExecutionError,
    VshStaleError,
    VshStateError,
)
from vsh.mcp import vsh_run


def test_bash_mcp_cli_host_flags_and_bounded_output(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    import json

    from fastmcp import FastMCP

    from vsh.cli import main
    from vsh.mcp.surface import register_vsh_surface

    monkeypatch.delenv("VSH_ENABLE_BASH", raising=False)
    disabled = FastMCP("disabled")
    register_vsh_surface(disabled)
    assert asyncio.run(disabled.list_tools())[0].parameters["properties"]["language"]["enum"] == [
        "monty"
    ]
    with pytest.raises(ValueError, match="not host-enabled"):
        vsh_run("true", language="bash", workspace_root=str(tmp_path))
    with pytest.raises(ValueError, match="unknown language"):
        vsh_run(
            "true", language=cast(Literal["monty", "bash"], "other"), workspace_root=str(tmp_path)
        )
    worker = os.environ.get(
        "VSH_BASH_WORKER", str(Path(sys.executable).with_name("vsh-bash-worker"))
    )
    monkeypatch.setenv("VSH_BASH_WORKER", worker)
    main(
        [
            "run",
            "--workspace",
            str(tmp_path),
            "--language",
            "bash",
            "--enable-bash",
            "--bash-worker",
            worker,
            "--code",
            "printf cli > cli.txt",
            "--mode",
            "auto",
        ]
    )
    assert json.loads(capsys.readouterr().out)["state"] == "committed"
    assert (tmp_path / "cli.txt").read_text() == "cli"
    enabled = FastMCP("enabled")
    register_vsh_surface(enabled)
    assert asyncio.run(enabled.list_tools())[0].parameters["properties"]["language"]["enum"] == [
        "monty",
        "bash",
    ]
    binary = b"\xff" * (65 * 1024)
    (tmp_path / "large.bin").write_bytes(binary)
    result = vsh_run(
        "cat large.bin; cat large.bin >&2", language="bash", workspace_root=str(tmp_path)
    )
    output = cast(dict[str, object], result["bash"])
    assert output["output_truncated"] is True
    assert result["stdout_truncated"] is True and result["stderr_truncated"] is True
    assert base64.b64decode(str(output["stdout"])) == binary[:65536]
    assert base64.b64decode(str(output["stderr"])) == binary[:65536]


def test_async_native_join_and_commit_arbitration(tmp_path: Path) -> None:
    from vsh._async import native_call
    from vsh._native import _Cancellation

    runtime = Runtime.open(tmp_path)

    async def scenario(commit: bool) -> None:
        ready = threading.Event()
        release = threading.Event()
        cleaning = threading.Event()
        finish_cleanup = threading.Event()
        token = _Cancellation()
        cleaned: list[str] = []
        preview = (
            runtime.preview("vsh_write('/workspace/entered.txt', 'committed')") if commit else None
        )

        def execute():
            if preview is not None:
                result = runtime._commit(preview.transaction, 0, token)
            else:
                result = runtime.preview("vsh_write('/workspace/cancelled.txt', 'unused')")
            ready.set()
            assert release.wait(5)
            return result

        def cleanup(receipt):
            cleaning.set()
            assert finish_cleanup.wait(5)
            runtime._cancel_receipt(receipt.transaction)
            cleaned.append(receipt.transaction)

        task = asyncio.create_task(native_call(execute, token, on_cancel=cleanup))
        assert await asyncio.to_thread(ready.wait, 5)
        task.cancel()
        await asyncio.sleep(0)
        task.cancel()
        release.set()
        if commit:
            result = await task
            assert result.state == "committed" and cleaned == []
        else:
            assert await asyncio.to_thread(cleaning.wait, 5)
            task.cancel()
            await asyncio.sleep(0)
            finish_cleanup.set()
            with pytest.raises(asyncio.CancelledError):
                await task
            assert len(cleaned) == 1
            # Identical source can be previewed again: no hidden ephemeral lease.
            retried = runtime.preview("vsh_write('/workspace/cancelled.txt', 'unused')")
            assert runtime.discard_preview(retried.transaction)

    asyncio.run(scenario(False))
    asyncio.run(scenario(True))
    assert (tmp_path / "entered.txt").read_text() == "committed"
    assert not (tmp_path / "cancelled.txt").exists()


def test_mcp_client_cancellation_cannot_commit_later(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    from fastmcp import Client, FastMCP
    from fastmcp.server.dependencies import get_context

    from vsh.mcp.native_tools import _PreparedRun
    from vsh.mcp.surface import register_vsh_surface

    monkeypatch.setenv("VSH_ENABLE_BASH", "1")
    started = threading.Event()
    joined = threading.Event()
    request_ids: list[str | int] = []
    original = _PreparedRun.execute

    def execute(self, cancellation):
        context = get_context().request_context
        assert context is not None
        request_ids.append(context.request_id)
        started.set()
        try:
            return original(self, cancellation)
        finally:
            joined.set()

    monkeypatch.setattr(_PreparedRun, "execute", execute)
    server = FastMCP("cancel-native-run")
    register_vsh_surface(server)

    async def scenario() -> None:
        async with Client(server) as client:
            task = asyncio.create_task(
                client.call_tool(
                    "vsh_run",
                    {
                        "code": "sleep 0.7; printf late > after.txt",
                        "language": "bash",
                        "workspace_root": str(tmp_path),
                        "mode": "auto",
                    },
                )
            )
            assert await asyncio.to_thread(started.wait, 5)
            await asyncio.sleep(0.1)
            # Cancelling a local Python await alone does not send an MCP
            # notification. Exercise the actual protocol cancellation boundary.
            await client.cancel(request_ids[0], reason="cancel regression")
            task.cancel()
            with pytest.raises(asyncio.CancelledError):
                await task
            assert await asyncio.to_thread(joined.wait, 5)
            # Cancellation was processed by the registered tool, not merely by
            # the transport. Native execution has joined before this assertion.
            assert not (tmp_path / "after.txt").exists()

    asyncio.run(scenario())


@pytest.mark.parametrize("resume", [False, True])
def test_mcp_cancellation_after_commit_entry_returns_actual_result(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, resume: bool
) -> None:
    from fastmcp import FastMCP

    from vsh.mcp.native_tools import _PreparedRun
    from vsh.mcp.surface import register_vsh_surface

    entered = threading.Event()
    release = threading.Event()
    original = _PreparedRun.execute

    def execute(self, cancellation):
        receipt = original(self, cancellation)
        assert cancellation.commit_entered
        entered.set()
        assert release.wait(5)
        return receipt

    arguments: dict[str, object] = {"workspace_root": str(tmp_path), "mode": "auto"}
    code = "vsh_write('/workspace/entered.txt', 'committed')"
    if resume:
        preview = vsh_run(code, workspace_root=str(tmp_path))
        arguments["transaction"] = preview["transaction"]
    else:
        arguments["code"] = code
    monkeypatch.setattr(_PreparedRun, "execute", execute)
    server = FastMCP("commit-arbitration")
    register_vsh_surface(server)

    async def scenario() -> None:
        tool = (await server.list_tools())[0]
        task = asyncio.create_task(tool.run(arguments))
        assert await asyncio.to_thread(entered.wait, 5)
        task.cancel()
        await asyncio.sleep(0)
        task.cancel()
        release.set()
        result = await task
        assert result.structured_content is not None
        assert result.structured_content["state"] == "committed"

    asyncio.run(scenario())
    assert (tmp_path / "entered.txt").read_text() == "committed"


def test_async_cancellation_consumes_native_failure() -> None:
    from vsh._async import native_call

    ready = threading.Event()
    release = threading.Event()

    def execute() -> None:
        ready.set()
        assert release.wait(5)
        raise RuntimeError("native failure racing cancellation")

    async def scenario() -> None:
        errors: list[dict[str, object]] = []
        asyncio.get_running_loop().set_exception_handler(lambda _loop, error: errors.append(error))
        task = asyncio.create_task(native_call(execute))
        assert await asyncio.to_thread(ready.wait, 5)
        task.cancel()
        await asyncio.sleep(0)
        task.cancel()
        release.set()
        with pytest.raises(asyncio.CancelledError):
            await task
        await asyncio.sleep(0)
        assert not errors

    asyncio.run(scenario())


@pytest.mark.parametrize("size", [3, 70 * 1024])
def test_cli_bash_failure_preserves_bounded_binary_diagnostics(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str], size: int
) -> None:
    import json

    from vsh.cli import main

    monkeypatch.setenv("VSH_ENABLE_BASH", "1")
    binary = b"\xff\x00\xfe" * size
    (tmp_path / "binary.bin").write_bytes(binary)
    with pytest.raises(SystemExit) as failure:
        main(
            [
                "run",
                "--workspace",
                str(tmp_path),
                "--language",
                "bash",
                "--mode",
                "auto",
                "--code",
                "cat binary.bin; cat binary.bin >&2; printf partial > partial.txt; exit 7",
            ]
        )
    assert failure.value.code == 7
    output = capsys.readouterr()
    assert output.out == ""
    diagnostics = json.loads(output.err)
    assert diagnostics["state"] == "failed" and diagnostics["committable"] is False
    assert diagnostics["exit_code"] == 7 and diagnostics["encoding"] == "base64"
    assert base64.b64decode(diagnostics["stdout"]) == binary[:65536]
    assert base64.b64decode(diagnostics["stderr"]) == binary[:65536]
    assert diagnostics["output_truncated"] is (len(binary) > 65536)
    assert diagnostics["changes_complete"] is True
    assert any(item["path"] == "partial.txt" for item in diagnostics["changes"])
    assert "transaction" not in diagnostics
    assert not (tmp_path / "partial.txt").exists()


def test_cli_bash_profile_failure_has_nonzero_status(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, capsys: pytest.CaptureFixture[str]
) -> None:
    import json

    from vsh.cli import main

    monkeypatch.setenv("VSH_ENABLE_BASH", "1")
    with pytest.raises(SystemExit) as failure:
        main(["run", "--workspace", str(tmp_path), "--language", "bash", "--code", "ln -s x link"])
    assert failure.value.code == 1
    diagnostics = json.loads(capsys.readouterr().err)
    assert diagnostics["state"] == "failed"
    assert diagnostics["committable"] is False


def test_hook_preview_conflict_and_resolve_failure_remain_non_mutating(tmp_path: Path) -> None:
    runtime: HookedRuntime

    def handler(event: RequestEvent) -> HookDecision:
        preparation = runtime.native.prepare_commit(event.transaction)
        runtime.native.fail_hook(preparation)
        return HookDecision.approve("a now-invalid preparation must not commit")

    runtime = HookedRuntime.open(tmp_path, hook_handler=handler, hook_scope=HookScope.ALL_REQUESTS)
    with pytest.raises(TypeError, match="only valid"):
        cast(Callable[..., Receipt], runtime.preview)(RunRequest("42"), intent="ambiguous override")
    preview = runtime.preview("vsh_write('/workspace/invalidated.txt', 'no')")
    with pytest.raises(VshStateError):
        asyncio.run(runtime.acommit(preview.transaction))
    assert runtime.transaction_state(preview.transaction) == "pending_approval"
    assert not (tmp_path / "invalidated.txt").exists()


def test_cancelled_hook_preparation_revokes_durable_auto_approval(tmp_path: Path) -> None:
    ready = threading.Event()
    release = threading.Event()
    native = Runtime.open(tmp_path, hook_id="cancel-preparation", hook_scope=HookScope.ALL_REQUESTS)
    preview = native.preview("vsh_write('/workspace/prepared.txt', 'no')")

    class GatedRuntime:
        def prepare_commit(self, transaction):
            preparation = native.prepare_commit(transaction)
            ready.set()
            assert release.wait(5)
            return preparation

        def fail_hook(self, preparation):
            return native.fail_hook(preparation)

    runtime = HookedRuntime(
        cast(Runtime, GatedRuntime()), lambda _: HookDecision.approve("must not be reached")
    )

    async def scenario() -> None:
        task = asyncio.create_task(runtime.acommit(preview.transaction))
        assert await asyncio.to_thread(ready.wait, 5)
        task.cancel()
        release.set()
        with pytest.raises(asyncio.CancelledError):
            await task

    asyncio.run(scenario())
    assert native.transaction_state(preview.transaction) == "pending_approval"
    assert not (tmp_path / "prepared.txt").exists()


def test_cancelled_preview_cleanup_also_revokes_a_persisted_copy(tmp_path: Path) -> None:
    runtime = Runtime.open(tmp_path, hook_id="cancel-promoted", hook_scope=HookScope.ALL_REQUESTS)
    preview = runtime.preview("vsh_write('/workspace/promoted.txt', 'no')")
    runtime.prepare_commit(preview.transaction)
    runtime._cancel_receipt(preview.transaction)
    assert runtime.transaction_state(preview.transaction) == "pending_approval"
    assert not runtime.discard_preview(preview.transaction)
    assert not (tmp_path / "promoted.txt").exists()


@pytest.mark.skipif(os.name != "posix", reason="initial Bash profile requires a Unix host")
def test_bash_public_runtime_bytes_diagnostics_and_restart_hook(tmp_path: Path) -> None:
    (tmp_path / "binary.bin").write_bytes(b"\xff\x00\xfe")
    events: list[RequestEvent] = []

    def handler(event: RequestEvent) -> HookDecision:
        events.append(event)
        return HookDecision.approve("verified exact canonical changes")

    runtime = HookedRuntime.open(
        tmp_path,
        bash=BashConfig(),
        worker_path="/missing-monty",
        hook_handler=handler,
        hook_scope=HookScope.ALL_REQUESTS,
    )
    preview = runtime.preview(
        "cat binary.bin > result.bin; cat binary.bin; printf err >&2",
        language=Language.BASH,
        detail=ReceiptDetail.FULL,
    )
    assert isinstance(preview.result, BashResult)
    assert preview.language is Language.BASH
    assert preview.result.stdout == preview.stdout_bytes == b"\xff\x00\xfe"
    assert preview.result.stderr == preview.stderr_bytes == b"err"
    assert preview.stderr == "err"
    assert preview.output_bytes == 6 and preview.result_bytes == 4
    assert not (tmp_path / "result.bin").exists()
    preparation = runtime.native.prepare_commit(preview.transaction)
    del runtime
    reopened = HookedRuntime.open(
        tmp_path,
        bash=BashConfig(),
        worker_path="/missing-monty",
        hook_handler=handler,
        hook_scope=HookScope.ALL_REQUESTS,
    )
    committed = reopened.commit(preview.transaction).receipt
    assert committed.state == "committed"
    assert isinstance(committed.result, BashResult)
    assert committed.result.stdout == preview.result.stdout
    assert (tmp_path / "result.bin").read_bytes() == b"\xff\x00\xfe"
    assert events[0].schema_version == 2
    assert events[0].execution_context.language is Language.BASH
    assert events[0].execution_context.complete
    assert events[0].execution_context.evidence
    assert preparation.event is not None and preparation.event.event_id == events[0].event_id
    assert any(effect.origin == "bash_call" for effect in events[0].effects)
    with pytest.raises(VshBashError) as failed:
        reopened.run(
            RunRequest(
                "printf partial > partial.txt; cat binary.bin; printf problem >&2; exit 7",
                language=Language.BASH,
                mode=RunMode.AUTO,
            )
        )
    assert failed.value.diagnostics.kind == "exit"
    assert failed.value.diagnostics.exit_code == 7
    assert failed.value.diagnostics.stdout == b"\xff\x00\xfe"
    assert failed.value.diagnostics.stderr == b"problem"
    assert failed.value.diagnostics.changes[0].path == "partial.txt"
    assert failed.value.diagnostics.changes_complete
    assert not (tmp_path / "partial.txt").exists()


@pytest.mark.skipif(os.name != "posix", reason="initial Bash profile requires a Unix host")
def test_bash_enablement_limits_and_explicit_pending_approval(tmp_path: Path) -> None:
    disabled = Runtime.open(tmp_path)
    with pytest.raises(VshExecutionError, match="not enabled"):
        disabled.preview("true", language=Language.BASH)
    monty = disabled.preview("42")
    assert monty.language is Language.MONTY
    assert monty.stderr_bytes == b""
    assert disabled.preview("print('text')").stdout_bytes == b"text\n"
    with pytest.raises(TypeError, match="only valid"):
        disabled.preview(cast(str, RunRequest("42")), language=Language.BASH)
    with pytest.raises(ValueError, match="worker counts"):
        BashConfig(max_active_workers=0)
    runtime = Runtime.open(tmp_path, bash=BashConfig(limits=BashLimits(max_commands=2)))
    with pytest.raises(VshBashError):
        runtime.run(
            RunRequest(
                "printf a > partial.txt; true; true; true",
                language=Language.BASH,
                mode=RunMode.AUTO,
            )
        )
    assert not (tmp_path / "partial.txt").exists()
    strict = Runtime.open(tmp_path, policy="strict", bash=BashConfig())
    pending = strict.run(
        RunRequest("printf safe > reviewed.txt", language=Language.BASH, mode=RunMode.AUTO)
    )
    assert pending.state == "pending_approval"
    assert not (tmp_path / "reviewed.txt").exists()
    strict.approve(pending.transaction, "reviewer", 1, 1000)
    assert strict.commit(pending.transaction, 2).state == "committed"


@pytest.mark.parametrize("language", [Language.MONTY, Language.BASH])
def test_async_native_cancellation_joins_worker_and_never_detaches_commit(
    tmp_path: Path, language: Language
) -> None:
    if language is Language.BASH and os.name != "posix":
        pytest.skip("initial Bash profile requires Unix")
    from vsh._async import native_call
    from vsh._native import _Cancellation

    async def scenario() -> None:
        runtime = Runtime.open(tmp_path, bash=BashConfig() if language is Language.BASH else None)
        token = _Cancellation()
        code = (
            "printf partial > cancelled.txt; sleep 10; printf late > cancelled.txt"
            if language is Language.BASH
            else "while True:\n    pass"
        )
        request = RunRequest(
            code,
            language=language,
            mode=RunMode.AUTO,
            budget=ExecutionBudget(max_duration_ms=20_000),
        )
        task = asyncio.create_task(native_call(lambda: runtime._run(request, token), token))
        await asyncio.sleep(0.1)
        started = time.monotonic()
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
        assert time.monotonic() - started < 2
        assert not token.commit_entered
        assert not (tmp_path / "cancelled.txt").exists()
        assert runtime.preview("6 * 7").result == 42

    asyncio.run(scenario())


@pytest.mark.parametrize("limit", ["records", "bytes"])
def test_evidence_limits_are_terminal_and_auto_mode_never_commits_partial_work(
    tmp_path: Path, limit: str
) -> None:
    budget = ExecutionBudget(
        max_evidence_records=8 if limit == "records" else None,
        max_evidence_bytes=0 if limit == "bytes" else None,
    )
    assert budget.max_evidence_records == (8 if limit == "records" else 250_000)
    assert budget.max_evidence_bytes == (0 if limit == "bytes" else 64 * 1024 * 1024)
    runtime = Runtime.open(tmp_path)
    with pytest.raises(VshExecutionError, match="filesystem evidence"):
        runtime.run(
            RunRequest(
                "from pathlib import Path\n"
                "Path('/workspace/partial.txt').write_text('virtual only')\n"
                "for i in range(20):\n"
                "    try:\n"
                "        Path('/workspace/missing').stat()\n"
                "    except OSError:\n"
                "        pass\n"
                "'done'",
                mode=RunMode.AUTO,
                budget=budget,
            )
        )
    assert not (tmp_path / "partial.txt").exists()
    assert runtime.preview("6 * 7").result == 42


@pytest.mark.parametrize(
    "example",
    [
        "preview.py",
        "auto_commit.py",
        "strict_review.py",
        "budgeted_analysis.py",
        "workflows.py",
        "mcp_workflow.py",
        "cli_workflow.py",
    ],
)
def test_native_cookbook_examples_execute_their_contracts(example: str) -> None:
    source = Path(__file__).resolve().parents[1] / "examples" / "native" / example
    runpy.run_path(str(source), run_name="__main__")


def test_first_transaction_documentation_is_executable() -> None:
    source = Path(__file__).resolve().parents[1] / "docs/start/index.md"
    programs = re.findall(r"^```python\n(.*?)^```", source.read_text(), re.MULTILINE | re.DOTALL)
    assert len(programs) == 1
    exec(compile(programs[0], str(source), "exec"), {"__name__": "__documentation_example__"})


def test_pyo3_runtime_auto_commit_uses_the_native_core(tmp_path: Path) -> None:
    (tmp_path / "input.txt").write_text("hello\n")
    runtime = Runtime.open(tmp_path)
    receipt = runtime.run(
        RunRequest(
            """
from pathlib import Path
value = Path('/workspace/input.txt').read_text()
Path('/workspace/output.txt').write_text(value.upper())
len(value)
""",
            mode=RunMode.AUTO,
            detail=ReceiptDetail.FULL,
        )
    )

    assert receipt.state == "committed"
    assert receipt.decision == "auto_approved"
    assert receipt.committed is True
    assert receipt.changed_paths == 1
    assert receipt.changes == [("output.txt", "create")]
    assert receipt.result == 6
    assert receipt.result_repr == "6"
    assert receipt.commit_operations is not None
    assert receipt.verified_paths == 1
    assert (tmp_path / "output.txt").read_text() == "HELLO\n"
    assert set(receipt.timings_ns()) == {
        "bind_and_store",
        "commit",
        "diff",
        "execute",
        "policy",
        "snapshot",
        "total",
    }


def test_python_hook_returns_evidence_first_feedback_and_keeps_pending(tmp_path: Path) -> None:
    (tmp_path / "input.txt").write_text("source")
    events: list[RequestEvent] = []

    def handler(event: RequestEvent) -> HookDecision:
        events.append(event)
        return HookDecision.review(
            "output.txt copies sensitive-looking source data; main agent must confirm"
        )

    runtime = HookedRuntime.open(
        tmp_path,
        hook_handler=handler,
        hook_scope=HookScope.ALL_REQUESTS,
        hook_id="python-evidence-review",
    )
    preview = runtime.preview(
        RunRequest(
            "from pathlib import Path\n"
            "value = Path('/workspace/input.txt').read_text()\n"
            "Path('/workspace/output.txt').write_text(value)",
            intent="copy the source",
        )
    )
    resolution = runtime.commit(preview.transaction, now_unix_ms=100)

    assert resolution.receipt.state == "pending_approval"
    assert resolution.hook is not None
    assert resolution.hook.verdict == "review"
    assert "main agent" in resolution.hook.reason
    assert runtime.transaction_state(preview.transaction) == "pending_approval"
    assert not (tmp_path / "output.txt").exists()
    assert len(events) == 1
    event = events[0]
    assert event.intent == "copy the source"
    assert event.intent_digest is not None
    assert event.baseline == "auto_approved"
    assert event.policy_profile == "balanced"
    assert event.canonical_diff[0].path == "output.txt"
    assert event.canonical_diff[0].kind == "create"
    assert event.canonical_diff[0].before is None
    assert event.canonical_diff[0].after is not None
    assert event.canonical_diff[0].after.kind == "file"
    assert event.canonical_diff[0].after.content is not None
    assert any(effect.operation == "content_read" for effect in event.effects)
    assert any(effect.operation == "create" for effect in event.effects)
    assert event.read_bytes > 0
    assert event.write_bytes > 0
    assert event.evidence_complete is True
    assert event.evidence_truncated is False

    assert runtime.approve(preview.transaction, "main-agent", 110, 200) == "approved"
    committed = runtime.commit(preview.transaction, now_unix_ms=120)
    assert committed.receipt.state == "committed"
    assert committed.hook is None
    assert (tmp_path / "output.txt").read_text() == "source"


def test_python_async_hook_can_approve_read_only_request(tmp_path: Path) -> None:
    (tmp_path / "read.txt").write_text("bounded")
    events: list[RequestEvent] = []

    async def handler(event: RequestEvent) -> HookDecision:
        await asyncio.sleep(0)
        events.append(event)
        return HookDecision.approve("read set is bounded")

    runtime = HookedRuntime.open(
        tmp_path,
        hook_handler=handler,
        hook_scope=HookScope.ALL_REQUESTS,
    )
    receipt = asyncio.run(
        runtime.arun(
            RunRequest(
                "from pathlib import Path\nPath('/workspace/read.txt').read_text()",
                mode=RunMode.AUTO,
            )
        )
    )

    assert receipt.state == "committed"
    assert len(events) == 1
    assert events[0].canonical_diff == []
    assert any(effect.operation == "content_read" for effect in events[0].effects)


def test_sync_hook_rejects_awaitable_and_fails_closed(tmp_path: Path) -> None:
    async def handler(_event: RequestEvent) -> HookDecision:
        return HookDecision.approve("async")

    runtime = HookedRuntime.open(
        tmp_path,
        hook_handler=handler,
        hook_scope=HookScope.ALL_REQUESTS,
    )
    preview = runtime.preview(
        "from pathlib import Path\nPath('/workspace/deferred.txt').write_text('value')"
    )

    with pytest.raises(TypeError, match=r"use acommit\(\) or arun\(\)"):
        runtime.commit(preview.transaction, now_unix_ms=1)

    assert runtime.transaction_state(preview.transaction) == "pending_approval"
    assert not (tmp_path / "deferred.txt").exists()


def test_hooked_runtime_passthrough_paths_and_helpers(tmp_path: Path) -> None:
    calls = 0

    def handler(_event: RequestEvent) -> HookDecision:
        nonlocal calls
        calls += 1
        return HookDecision.follow_policy()

    runtime = HookedRuntime.open(tmp_path, hook_handler=handler)
    assert isinstance(runtime.native, Runtime)

    discarded = runtime.preview("41 + 1")
    assert runtime.discard_preview(discarded.transaction)

    preview = runtime.run(RunRequest("1", mode=RunMode.PREVIEW))
    assert preview.state == "auto_approved"
    assert runtime.discard_preview(preview.transaction)

    committed = runtime.run(
        RunRequest(
            "from pathlib import Path\nPath('/workspace/direct.txt').write_text('ok')",
            mode=RunMode.AUTO,
        ),
        now_unix_ms=10,
    )
    assert committed.state == "committed"
    assert calls == 0
    assert runtime.recover().conflicts == []


def test_async_hook_invalid_result_fails_closed(tmp_path: Path) -> None:
    runtime = HookedRuntime.open(
        tmp_path,
        hook_handler=lambda _event: cast(HookDecision, "invalid"),
        hook_scope=HookScope.ALL_REQUESTS,
    )
    preview = runtime.preview(
        "from pathlib import Path\nPath('/workspace/invalid.txt').write_text('value')"
    )

    with pytest.raises(TypeError, match="must return HookDecision"):
        asyncio.run(runtime.acommit(preview.transaction))

    assert runtime.transaction_state(preview.transaction) == "pending_approval"


def test_sync_hook_closes_only_real_coroutines(tmp_path: Path) -> None:
    class AwaitableDecision:
        def __await__(self):
            yield
            return HookDecision.follow_policy()

    runtime = HookedRuntime.open(
        tmp_path,
        hook_handler=lambda _event: cast(HookDecision, AwaitableDecision()),
        hook_scope=HookScope.ALL_REQUESTS,
    )
    preview = runtime.preview("42")

    with pytest.raises(TypeError, match=r"use acommit\(\) or arun\(\)"):
        runtime.commit(preview.transaction)

    assert runtime.transaction_state(preview.transaction) == "pending_approval"


def test_hooked_runtime_async_preview_does_not_invoke_handler(tmp_path: Path) -> None:
    calls = 0

    def handler(_event: RequestEvent) -> HookDecision:
        nonlocal calls
        calls += 1
        return HookDecision.follow_policy()

    runtime = HookedRuntime.open(
        tmp_path,
        hook_handler=handler,
        hook_scope=HookScope.ALL_REQUESTS,
    )
    receipt = asyncio.run(runtime.arun(RunRequest("42", mode=RunMode.PREVIEW)))

    assert receipt.state == "auto_approved"
    assert calls == 0
    assert runtime.discard_preview(receipt.transaction)


def test_pyo3_vsh_functions_and_pathlib_share_one_preview_overlay(tmp_path: Path) -> None:
    (tmp_path / "input.txt").write_text("Needle\n")
    runtime = Runtime.open(tmp_path)
    receipt = runtime.preview(
        r"""
from pathlib import Path

source = vsh_read('/workspace/input.txt')
vsh_mkdir('/workspace/generated')
vsh_write('/workspace/generated/result.txt', source.upper())
assert Path('/workspace/generated/result.txt').read_text() == 'NEEDLE\n'
changed = vsh_patch('/workspace/generated/result.txt', 'NEEDLE', 'Found')
paths = vsh_glob('**/*.txt', path='/workspace/generated', max_results=5)
hits = vsh_search('found', path='/workspace/generated', case_sensitive=False, max_results=5)
listed = vsh_list('/workspace/generated')
(source, changed, len(paths), hits[0]['line'], len(listed), vsh_read(paths[0]))
""",
        detail=ReceiptDetail.FULL,
    )

    assert receipt.result == ("Needle\n", 1, 1, 1, 1, "Found\n")
    assert receipt.os_calls == 9
    assert receipt.changes == [
        ("generated", "create"),
        ("generated/result.txt", "create"),
    ]
    assert not (tmp_path / "generated").exists()


def test_single_mcp_tool_promotes_one_exact_native_preview(tmp_path: Path) -> None:
    code = """
vsh_write('/workspace/from-mcp.txt', 'native')
{'engine': 'rust', 'files': 1}
"""
    preview = vsh_run(code, workspace_root=str(tmp_path), mode="preview", detail="full")

    assert preview["state"] == "auto_approved"
    assert preview["changes"] == [{"path": "from-mcp.txt", "kind": "create"}]
    assert not (tmp_path / "from-mcp.txt").exists()

    committed = vsh_run(
        transaction=str(preview["transaction"]),
        workspace_root=str(tmp_path),
        mode="auto",
    )

    assert committed["state"] == "committed"
    commit = cast(dict[str, object], committed["commit"])
    assert commit["committed"] is True
    assert (tmp_path / "from-mcp.txt").read_text() == "native"


def test_fastmcp_server_exposes_exactly_one_normal_tool() -> None:
    from vsh.mcp.server import mcp

    tools = asyncio.run(mcp.list_tools())

    assert [tool.name for tool in tools] == ["vsh_run"]


def test_pyo3_strict_preview_approval_and_commit_are_one_bound_artifact(
    tmp_path: Path,
) -> None:
    runtime = Runtime.open(tmp_path, policy="strict")
    preview = runtime.preview(
        RunRequest("from pathlib import Path\nPath('/workspace/approved.txt').write_text('yes')")
    )

    assert preview.state == "pending_approval"
    assert preview.decision == "pending_approval"
    assert preview.risk_flags == ["mutation"]
    assert not (tmp_path / "approved.txt").exists()
    assert runtime.approve(preview.transaction, "test-principal", 10, 20) == "approved"

    committed = runtime.commit(preview.transaction, 11)
    assert committed.state == "committed"
    assert (tmp_path / "approved.txt").read_text() == "yes"


@pytest.mark.parametrize("language", [Language.MONTY, Language.BASH])
@pytest.mark.parametrize("filename", ["*.key.key", ".env.**", "nested/.env.**/token"])
@pytest.mark.skipif(os.name != "posix", reason="literal '*' host filenames require POSIX")
def test_literal_star_secret_names_are_denied_in_both_frontends(
    tmp_path: Path, language: Language, filename: str
) -> None:
    secret = tmp_path / filename
    secret.parent.mkdir(parents=True, exist_ok=True)
    secret.write_text("mock-secret")
    runtime = Runtime.open(tmp_path, bash=BashConfig() if language is Language.BASH else None)
    if language is Language.BASH:
        code = f"cat '{filename}' || true; printf blocked > output.txt"
    else:
        code = f"""
from pathlib import Path
try:
    Path('/workspace/{filename}').read_text()
except PermissionError:
    pass
vsh_write('/workspace/output.txt', 'blocked')
"""
    receipt = runtime.run(RunRequest(code, language=language, mode=RunMode.AUTO))
    assert receipt.state == "denied"
    assert "mock-secret" not in receipt.stdout
    assert not (tmp_path / "output.txt").exists()
    assert secret.read_text() == "mock-secret"


def test_changed_policy_requires_fresh_preview_before_python_approval(tmp_path: Path) -> None:
    runtime = Runtime.open(tmp_path, policy="strict")
    preview = runtime.preview("vsh_write('/workspace/output.txt', 'safe')")
    del runtime
    changed = Runtime.open(tmp_path, policy="paranoid")
    with pytest.raises(VshStateError, match="policy changed.*fresh preview"):
        changed.approve(preview.transaction, "reviewer", 1, 100)
    with pytest.raises(VshStateError, match="policy changed.*fresh preview"):
        changed.prepare_commit(preview.transaction)
    with pytest.raises(VshStateError, match="policy changed.*fresh preview"):
        changed.commit(preview.transaction, 2)
    assert not (tmp_path / "output.txt").exists()
    fresh = changed.preview("vsh_write('/workspace/output.txt', 'safe')")
    assert fresh.transaction != preview.transaction
    changed.approve(fresh.transaction, "reviewer", 1, 100)
    assert changed.commit(fresh.transaction, 2).state == "committed"


def test_pyo3_preview_accepts_source_code_with_keyword_configuration(tmp_path: Path) -> None:
    runtime = Runtime.open(tmp_path)
    preview = runtime.preview(
        "from pathlib import Path\nPath('/workspace/direct.txt').write_text('yes')",
        intent="Exercise the direct preview overload",
        detail=ReceiptDetail.FULL,
        budget=ExecutionBudget(max_program_bytes=1024),
    )

    assert preview.state == "auto_approved"
    assert preview.changes == [("direct.txt", "create")]
    assert not (tmp_path / "direct.txt").exists()


def test_pyo3_preview_preserves_the_request_overload(tmp_path: Path) -> None:
    runtime = Runtime.open(tmp_path)

    preview = runtime.preview(request=RunRequest("{'answer': 42}"))

    assert preview.result == {"answer": 42}


def test_pyo3_preview_rejects_ambiguous_or_invalid_inputs(tmp_path: Path) -> None:
    runtime = Runtime.open(tmp_path)

    with pytest.raises(TypeError, match="only valid when preview.*receives source code"):
        runtime.preview(cast(str, RunRequest("42")), detail=ReceiptDetail.FULL)
    with pytest.raises(TypeError, match="requires a RunRequest or source-code str"):
        runtime.preview(cast(str, 42))


def test_pyo3_stale_error_never_applies_virtual_output(tmp_path: Path) -> None:
    source = tmp_path / "input.txt"
    source.write_text("before")
    runtime = Runtime.open(tmp_path)
    preview = runtime.preview(
        RunRequest(
            """
from pathlib import Path
value = Path('/workspace/input.txt').read_text()
Path('/workspace/output.txt').write_text(value)
"""
        )
    )
    source.write_text("external")

    with pytest.raises(VshStaleError):
        runtime.commit(preview.transaction, 0)

    assert source.read_text() == "external"
    assert not (tmp_path / "output.txt").exists()


def test_pyo3_budget_failure_has_no_host_effect(tmp_path: Path) -> None:
    runtime = Runtime.open(tmp_path)
    request = RunRequest(
        "from pathlib import Path\nPath('/workspace/nope.txt').write_text('too late')",
        mode=RunMode.AUTO,
        budget=ExecutionBudget(max_program_bytes=4),
    )

    with pytest.raises(VshExecutionError, match="program bytes limit exceeded"):
        runtime.run(request)

    assert not (tmp_path / "nope.txt").exists()


def test_pyo3_result_is_a_native_python_object_without_json_round_trip(tmp_path: Path) -> None:
    runtime = Runtime.open(tmp_path)
    receipt = runtime.preview(RunRequest("{'answer': 42, 'items': [True, None, b'raw']}"))

    assert receipt.result == {"answer": 42, "items": [True, None, b"raw"]}
    assert isinstance(receipt.result, dict)


def test_pyo3_can_discard_a_process_local_preview_without_host_effect(tmp_path: Path) -> None:
    runtime = Runtime.open(tmp_path)
    preview = runtime.preview(
        RunRequest("from pathlib import Path\nPath('/workspace/discarded.txt').write_text('no')")
    )

    assert runtime.discard_preview(preview.transaction) is True
    assert runtime.discard_preview(preview.transaction) is False
    assert not (tmp_path / "discarded.txt").exists()


def test_pyo3_pending_result_survives_restart_without_losing_type(tmp_path: Path) -> None:
    runtime = Runtime.open(tmp_path, policy="strict")
    preview = runtime.preview(
        RunRequest(
            """
from pathlib import Path
Path('/workspace/restarted.txt').write_text('durable')
{'answer': 42, 'items': [True, None, b'raw']}
"""
        )
    )
    transaction = preview.transaction
    del runtime

    restarted = Runtime.open(tmp_path, policy="strict")
    assert restarted.approve(transaction, "restart-test", 10, 20) == "approved"
    committed = restarted.commit(transaction, 11)

    assert committed.result == {"answer": 42, "items": [True, None, b"raw"]}
    assert isinstance(committed.result, dict)
    assert (tmp_path / "restarted.txt").read_text() == "durable"


def test_pyo3_rejects_data_directory_inside_untrusted_workspace(tmp_path: Path) -> None:
    data_directory = tmp_path / "unprotected-data"

    with pytest.raises(ValueError, match="must be disjoint"):
        Runtime.open(tmp_path, data_directory=data_directory)

    assert not data_directory.exists()


@pytest.mark.skipif(sys.platform == "win32", reason="uses POSIX flock as a blocking witness")
def test_pyo3_runtime_open_releases_the_gil_while_waiting_for_store_lock(
    tmp_path: Path,
) -> None:
    runtime = Runtime.open(tmp_path)
    del runtime
    lock_path = tmp_path / ".vsh-runtime/data/transactions.lock"
    holder = subprocess.Popen(
        [
            sys.executable,
            "-c",
            (
                "import fcntl, sys, time; "
                "f = open(sys.argv[1], 'r+b'); "
                "fcntl.flock(f, fcntl.LOCK_EX); "
                "print('locked', flush=True); "
                "time.sleep(0.75)"
            ),
            str(lock_path),
        ],
        stdout=subprocess.PIPE,
        text=True,
    )
    assert holder.stdout is not None
    assert holder.stdout.readline() == "locked\n"

    start = threading.Event()
    progressed = threading.Event()

    def mark_progress() -> None:
        start.wait()
        time.sleep(0.05)
        progressed.set()

    witness = threading.Thread(target=mark_progress)
    witness.start()
    try:
        start.set()
        started = time.monotonic()
        reopened = Runtime.open(tmp_path)
        elapsed = time.monotonic() - started
        progressed_before_return = progressed.is_set()
        del reopened
    finally:
        witness.join(timeout=2)
        holder.wait(timeout=2)

    assert elapsed >= 0.4
    assert progressed_before_return
