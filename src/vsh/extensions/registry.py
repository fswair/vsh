from __future__ import annotations as _annotations

from dataclasses import dataclass, field

from .protocols import ContentHydrator, SemanticAnalyzer, ShadowWorkspaceRunner

__all__ = ("ExtensionRegistry", "extensions")


@dataclass
class ExtensionRegistry:
    content_hydrator: ContentHydrator | None = None
    semantic_analyzers: list[SemanticAnalyzer] = field(default_factory=list)
    shadow_workspace_runners: list[ShadowWorkspaceRunner] = field(default_factory=list)


extensions = ExtensionRegistry()
