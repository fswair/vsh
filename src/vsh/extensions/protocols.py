from __future__ import annotations as _annotations

from typing import TYPE_CHECKING, Protocol

from vsh.snapshot.models import WorkspaceSnapshot

if TYPE_CHECKING:
    from vsh.plans.approval_models import ApprovalContext, ApproveItem

__all__ = (
    "ApprovalHandler",
    "ContentHydrator",
    "SemanticAnalyzer",
    "ShadowWorkspaceRunner",
)


class ApprovalHandler(Protocol):
    def __call__(self, ctx: ApprovalContext, item: ApproveItem) -> None: ...


class ContentHydrator(Protocol):
    def hydrate(self, path: str, content_ref: str | None) -> bytes | None: ...


class SemanticAnalyzer(Protocol):
    def analyze(self, snapshot: WorkspaceSnapshot, touched_paths: list[str]) -> list[str]: ...


class ShadowWorkspaceRunner(Protocol):
    def verify(self, snapshot: WorkspaceSnapshot, touched_paths: list[str]) -> list[str]: ...
