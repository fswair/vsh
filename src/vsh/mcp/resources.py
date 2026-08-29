from __future__ import annotations as _annotations

from typing import Any

from fastmcp.resources.function_resource import resource

from vsh.plans.store import plan_store
from vsh.registry import get_schema, registry
from vsh.runtime import runtime
from vsh.snapshot.builder import snapshot_workspace as build_snapshot_workspace
from vsh.snapshot.projection import project_snapshot


@resource("workspace://snapshot/current")
def workspace_snapshot_current() -> dict[str, Any]:
    try:
        snapshot = runtime.get_snapshot()
    except KeyError:
        snapshot = build_snapshot_workspace(".")
    return snapshot.model_dump()


@resource("workspace://projection/current")
def workspace_projection_current() -> dict[str, Any]:
    try:
        snapshot = runtime.get_snapshot()
    except KeyError:
        snapshot = build_snapshot_workspace(".")
    return project_snapshot(snapshot)


@resource("commands://spec/{name}")
def command_spec(name: str) -> dict[str, Any]:
    spec = registry[name]
    return {
        "spec": spec.model_dump(),
        "schema": get_schema(name),
    }


@resource("simulations://{plan_id}")
def simulation_record(plan_id: str) -> dict[str, Any]:
    record = plan_store.get(plan_id)
    payload = record.model_dump()
    return payload
