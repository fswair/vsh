from __future__ import annotations as _annotations

from pydantic import BaseModel, ConfigDict

from .resolver import ensure_safe_workspace_root, resolve_workspace_path


class SessionState(BaseModel):
    model_config = ConfigDict(extra="forbid")

    workspace_root: str
    cwd_logical: str
    cwd_physical: str
    oldpwd: str | None = None

    @classmethod
    def from_workspace_root(cls, workspace_root: str, cwd: str | None = None) -> SessionState:
        root = ensure_safe_workspace_root(workspace_root)
        current = root if cwd is None else resolve_workspace_path(root, cwd)
        return cls(
            workspace_root=root,
            cwd_logical=current,
            cwd_physical=current,
            oldpwd=None,
        )

    def with_cwd(self, cwd: str) -> SessionState:
        return self.model_copy(
            update={
                "oldpwd": self.cwd_logical,
                "cwd_logical": cwd,
                "cwd_physical": cwd,
            }
        )
