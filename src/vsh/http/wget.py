from __future__ import annotations as _annotations

from pathlib import Path
from urllib.parse import urlparse

__all__ = ("default_wget_output_name",)


def default_wget_output_name(url: str) -> str:
    """Derive wget's default on-disk filename from a URL."""
    path = urlparse(url).path
    name = Path(path).name
    return name or "index.html"
