from __future__ import annotations as _annotations

IGNORED_DIRECTORIES = frozenset(
    {
        ".git",
        ".pytest_cache",
        ".ruff_cache",
        "__pycache__",
        ".mypy_cache",
        ".venv",
        "node_modules",
        "dist",
        "build",
        "target",
    }
)

__all__ = ("IGNORED_DIRECTORIES",)
