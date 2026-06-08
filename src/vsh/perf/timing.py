from __future__ import annotations as _annotations

import time
from dataclasses import dataclass
from typing import Final

__all__ = (
    "CommandTimings",
    "elapsed_ms",
    "perf_counter_ns",
    "stamp_execution_time",
)

_NS_PER_MS: Final[float] = 1_000_000.0


def perf_counter_ns() -> int:
    """Return a monotonic high-resolution timestamp in nanoseconds."""
    return time.perf_counter_ns()


def elapsed_ms(start_ns: int, end_ns: int | None = None) -> float:
    """Convert a perf_counter_ns interval to milliseconds."""
    end = perf_counter_ns() if end_ns is None else end_ns
    return (end - start_ns) / _NS_PER_MS


def stamp_execution_time(effects: object, start_ns: int) -> object:
    """Attach execution_time_ms to an ActualEffects instance."""
    from vsh.effects import ActualEffects

    if not isinstance(effects, ActualEffects):
        msg = f"expected ActualEffects, got {type(effects).__name__}"
        raise TypeError(msg)
    return effects.model_copy(update={"execution_time_ms": elapsed_ms(start_ns)})


@dataclass(frozen=True, slots=True)
class CommandTimings:
    """Breakdown of command pipeline timings in milliseconds."""

    total_ms: float
    revalidation_ms: float = 0.0
    apply_ms: float = 0.0
    simulation_ms: float = 0.0
