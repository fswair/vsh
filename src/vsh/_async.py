"""Join native work on cancellation; never leave a detached commit running."""

from __future__ import annotations

import asyncio
from collections.abc import Callable
from typing import TypeVar

from ._native import _Cancellation

_T = TypeVar("_T")


async def native_call(
    call: Callable[[], _T],
    cancellation: _Cancellation | None = None,
    *,
    on_cancel: Callable[[_T], object] | None = None,
) -> _T:
    """Offload blocking native work and reconcile its actual commit outcome."""

    token = cancellation if cancellation is not None else _Cancellation()
    task = asyncio.create_task(asyncio.to_thread(call))
    interrupted: asyncio.CancelledError | None = None
    while not task.done():
        try:
            # wait() never cancels its input tasks and leaves exception retrieval
            # to this owner, including when cancellation races a native failure.
            await asyncio.wait({task})
        except asyncio.CancelledError as error:
            interrupted = error
            token.cancel()
    if interrupted is not None and not token.commit_entered:
        # Consume worker exceptions after joining, so cancellation cannot leak a
        # background task or falsely claim an entered commit was rolled back.
        error = task.exception()
        if error is None and on_cancel is not None:
            cleanup = asyncio.create_task(asyncio.to_thread(on_cancel, task.result()))
            while not cleanup.done():
                try:
                    await asyncio.wait({cleanup})
                except asyncio.CancelledError:
                    continue
            cleanup.result()
        raise interrupted
    return task.result()
