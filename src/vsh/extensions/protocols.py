from __future__ import annotations as _annotations

from typing import Protocol

from vsh.snapshot.models import WorkspaceSnapshot

__all__ = (
    "ContentHydrator",
    "SemanticAnalyzer",
    "ShadowWorkspaceRunner",
)


class ContentHydrator(Protocol):
    def hydrate(self, path: str, content_ref: str | None) -> bytes | None: ...


class SemanticAnalyzer(Protocol):
    def analyze(self, snapshot: WorkspaceSnapshot, touched_paths: list[str]) -> list[str]: ...


class ShadowWorkspaceRunner(Protocol):
    def verify(self, snapshot: WorkspaceSnapshot, touched_paths: list[str]) -> list[str]: ...
