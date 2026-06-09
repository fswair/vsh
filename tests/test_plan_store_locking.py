from __future__ import annotations as _annotations

import threading

from vsh.plans.store import PlanStore
from vsh.schemas import PwdCommand
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace


def test_plan_store_mark_executed_sets_timestamp(tmp_path) -> None:  # type: ignore[no-untyped-def]
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(PwdCommand(), snapshot)
    store = PlanStore()
    store.save(result, snapshot.snapshot_id, path_fingerprints={}, plan_fingerprint="fp")
    token = store.approve(result.plan_id)
    record = store.mark_executed(token.token)
    assert record.executed_at_ns is not None
    try:
        store.get_by_token(token.token)
    except KeyError:
        pass
    else:
        raise AssertionError("approval token should be consumed after execution")


def test_plan_store_consumes_token_after_execution(tmp_path) -> None:  # type: ignore[no-untyped-def]
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    result = simulate_command(PwdCommand(), snapshot)
    store = PlanStore()
    store.save(result, snapshot.snapshot_id, path_fingerprints={}, plan_fingerprint="fp")
    token = store.approve(result.plan_id)
    store.mark_executed(token.token)
    try:
        store.get_by_token(token.token)
    except KeyError:
        return
    raise AssertionError("consumed token must not be replayable")


def test_plan_store_is_thread_safe_for_save(tmp_path) -> None:  # type: ignore[no-untyped-def]
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))
    store = PlanStore()

    def worker() -> None:
        result = simulate_command(PwdCommand(), snapshot)
        store.save(
            result, snapshot.snapshot_id, path_fingerprints={}, plan_fingerprint=result.plan_id
        )

    threads = [threading.Thread(target=worker) for _ in range(8)]
    for thread in threads:
        thread.start()
    for thread in threads:
        thread.join()
    assert len(store.plans) == 8
