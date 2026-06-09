from __future__ import annotations as _annotations

import os
import re
import signal
from collections.abc import Iterator
from contextlib import contextmanager

__all__ = (
    "grep_max_file_bytes",
    "grep_max_matches",
    "grep_regex_timeout_secs",
    "read_max_file_bytes",
)


def read_max_file_bytes() -> int:
    raw = os.environ.get("VSH_READ_MAX_BYTES", "1048576")
    try:
        return max(1, int(raw))
    except ValueError:
        return 1_048_576


def grep_max_file_bytes() -> int:
    raw = os.environ.get("VSH_GREP_MAX_FILE_BYTES", str(read_max_file_bytes()))
    try:
        return max(1, int(raw))
    except ValueError:
        return read_max_file_bytes()


def grep_max_matches() -> int:
    raw = os.environ.get("VSH_GREP_MAX_MATCHES", "10000")
    try:
        return max(1, int(raw))
    except ValueError:
        return 10_000


def grep_regex_timeout_secs() -> float:
    raw = os.environ.get("VSH_GREP_REGEX_TIMEOUT_SECS", "2.0")
    try:
        return max(0.0, float(raw))
    except ValueError:
        return 2.0


@contextmanager
def regex_timeout(seconds: float) -> Iterator[None]:
    if seconds <= 0 or not hasattr(signal, "SIGALRM"):
        yield
        return

    def _handler(_signum: int, _frame: object) -> None:
        msg = "regex execution timed out"
        raise TimeoutError(msg)

    previous = signal.signal(signal.SIGALRM, _handler)
    signal.setitimer(signal.ITIMER_REAL, seconds)
    try:
        yield
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, previous)


def compile_grep_pattern(
    pattern: str,
    *,
    ignore_case: bool,
    fixed_strings: bool,
    extended_regexp: bool,
) -> re.Pattern[str] | None:
    if fixed_strings:
        return None
    flags = re.IGNORECASE if ignore_case else 0
    if extended_regexp:
        flags |= re.VERBOSE
    with regex_timeout(grep_regex_timeout_secs()):
        return re.compile(pattern, flags)
