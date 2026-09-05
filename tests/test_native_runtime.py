from __future__ import annotations

import asyncio
import re
import runpy
import subprocess
import sys
import threading
import time
from pathlib import Path
from typing import cast

import pytest

from vsh import (
    ExecutionBudget,
    ReceiptDetail,
    RunMode,
    RunRequest,
    Runtime,
    VshExecutionError,
    VshStaleError,
)
from vsh.mcp import vsh_run


@pytest.mark.parametrize("example", ["workflows.py", "mcp_workflow.py", "cli_workflow.py"])
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
