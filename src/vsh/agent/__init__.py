from __future__ import annotations as _annotations

import importlib
from typing import TYPE_CHECKING

from .deps import VshAgentDeps

if TYPE_CHECKING:
    from .toolset import create_vsh_function_toolset

__all__ = (
    "VshAgentDeps",
    "create_vsh_function_toolset",
)

_AGENT_IMPORT_ERROR = (
    "pydantic-ai is required for vsh.agent. "
    "Install with: `uv sync`, `pip install 'vsh[agent]'`, or `uv pip install -e '.[agent]'`."
)


def _load_create_vsh_function_toolset() -> object:
    try:
        module = importlib.import_module(".toolset", __name__)
    except ModuleNotFoundError as exc:
        if exc.name == "pydantic_ai":
            raise ImportError(_AGENT_IMPORT_ERROR) from exc
        raise
    return module.create_vsh_function_toolset


def __getattr__(name: str) -> object:
    if name == "create_vsh_function_toolset":
        return _load_create_vsh_function_toolset()
    msg = f"module {__name__!r} has no attribute {name!r}"
    raise AttributeError(msg)
