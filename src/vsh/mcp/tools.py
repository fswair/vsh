from __future__ import annotations as _annotations

import os
from typing import Any

from vsh.execute import execute_approved as execute_recorded_plan
from vsh.plans import approve_plan
from vsh.registry import get_schema as registry_get_schema
from vsh.registry import registrations
from vsh.registry import search as registry_search
from vsh.runtime import runtime
from vsh.schemas import CommandSpec
from vsh.simulate.engine import simulate_command
from vsh.snapshot.builder import snapshot_workspace as build_snapshot_workspace


def search(query: str) -> list[CommandSpec]:
    """Find vsh command specs by command name, alias, tag, or description."""
    return registry_search(query)


def get_schema(name: str) -> dict[str, Any]:
    """Return the JSON schema for a vsh structured command."""
    return registry_get_schema(name)


def snapshot_workspace(workspace_root: str | None = None, cwd: str | None = None) -> dict[str, Any]:
    """Create and persist a workspace snapshot graph."""
    root = workspace_root or os.getcwd()
    snapshot = build_snapshot_workspace(root, cwd=cwd)
    root_node = snapshot.nodes.get(snapshot.session.workspace_root)
    return {
        "snapshot_id": snapshot.snapshot_id,
        "session": snapshot.session.model_dump(),
        "generated_at_ns": snapshot.generated_at_ns,
        "node_count": len(snapshot.nodes),
        "root": root_node.model_dump() if root_node is not None else None,
    }


def simulate(tool_name: str, snapshot_id: str, params: dict[str, Any]) -> dict[str, Any]:
    """Simulate a structured command against a workspace snapshot."""
    registration = registrations[tool_name]
    snapshot = runtime.get_snapshot(snapshot_id)
    command = registration.schema_model(**params)
    result = simulate_command(command, snapshot)
    return result.model_dump()


def approve(plan_id: str) -> dict[str, Any]:
    """Approve a persisted simulation plan."""
    token = approve_plan(plan_id)
    return token.model_dump()


def execute_approved(approval_token: str) -> dict[str, Any]:
    """Execute an approved plan."""
    result = execute_recorded_plan(approval_token)
    return result.model_dump()
