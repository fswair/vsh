from __future__ import annotations as _annotations

from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass(frozen=True, slots=True)
class BenchmarkCase:
    name: str
    native_shell: str | None
    build_vsh_command: Callable[[Path], Any]
    prepare: Callable[[Path], None] | None = None
    native_note: str | None = None


@dataclass(frozen=True, slots=True)
class BenchmarkStats:
    name: str
    mode: str
    iterations: int
    median_ms: float
    min_ms: float
    max_ms: float
    mean_ms: float
    stdev_ms: float
    samples_ms: tuple[float, ...]
