from __future__ import annotations as _annotations

import os
from collections.abc import Iterator
from pathlib import Path

import pytest

from vsh.snapshot.builder import _build_nodes, node_for_path, snapshot_workspace


def test_build_nodes_adds_root_when_walk_is_empty(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    def empty_walk(_root: Path) -> Iterator[tuple[str, list[str], list[str]]]:
        return iter([])

    monkeypatch.setattr(os, "walk", lambda root: empty_walk(Path(root)))
    nodes = _build_nodes(workspace)

    assert str(workspace.resolve()) in nodes


def test_build_nodes_includes_workspace_root(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()

    nodes = _build_nodes(workspace)

    assert str(workspace.resolve()) in nodes


def test_node_for_path_detects_symlink(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    target = workspace / "target.txt"
    target.write_text("x\n", encoding="utf-8")
    link = workspace / "link.txt"
    link.symlink_to(target)

    node = node_for_path(link)

    assert node.kind == "symlink"


def test_snapshot_workspace_indexes_nested_files(tmp_path: Path) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    nested = workspace / "src"
    nested.mkdir()
    (nested / "main.py").write_text("print('hi')\n", encoding="utf-8")

    snapshot = snapshot_workspace(str(workspace), cwd=str(workspace))

    assert str((nested / "main.py").resolve()) in snapshot.nodes
    assert snapshot.nodes[str(nested.resolve())].children
