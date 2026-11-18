"""What every async class shares: how long one wait lasts, and the threads the
waits run on."""

import asyncio
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor
from typing import TypeVar

# The longest one wait in the worker thread lasts: 100 ms.
_PIECE_SECONDS = 0.1

# Waits at once on one object: a write, a wait for playout and a close, say.
# Threads are made only as calls need them.
_MOST_AT_ONCE = 4

T = TypeVar("T")


class _Waits:
    """One async object's own threads for its waits. Not the app's default thread
    pool (``asyncio.to_thread``): an app that keeps that busy with its own
    blocking work would otherwise starve the audio (a speaker read and a
    microphone write got nothing done until the pool freed up)."""

    def __init__(self) -> None:
        self._pool: ThreadPoolExecutor | None = None

    async def run(self, call: Callable[..., T], *args: object) -> T:
        """Runs ``call(*args)`` on one of these threads and returns its result."""
        if self._pool is None:
            self._pool = ThreadPoolExecutor(
                max_workers=_MOST_AT_ONCE, thread_name_prefix="pattie-aio"
            )
        return await asyncio.get_running_loop().run_in_executor(self._pool, call, *args)

    def shut_down(self) -> None:
        """Lets the threads end once their calls return; a later call makes new
        ones, so a closed object still answers as closed."""
        pool, self._pool = self._pool, None
        if pool is not None:
            pool.shutdown(wait=False)
