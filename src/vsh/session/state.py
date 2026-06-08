from __future__ import annotations as _annotations

from pathlib import Path

from pydantic import BaseModel, ConfigDict

from .resolver import ensure_safe_workspace_root


class SessionState(BaseModel):
    model_config = ConfigDict(extra="forbid")

    workspace_root: str
    cwd_logical: str
    cwd_physical: str
    oldpwd: str | None = None

    @classmethod
    def from_workspace_root(cls, workspace_root: str, cwd: str | None = None) -> SessionState:
        root = Path(ensure_safe_workspace_root(workspace_root))
        current = root if cwd is None else Path(cwd).expanduser().resolve()
        return cls(
            workspace_root=str(root),
            cwd_logical=str(current),
            cwd_physical=str(current),
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
