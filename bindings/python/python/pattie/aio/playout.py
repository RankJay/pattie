"""AsyncPlayout: a Playout whose waits are awaitable."""

from collections.abc import Buffer
from types import TracebackType
from typing import Self

from pattie._native import (
    Interrupted,
    Playout,
)
from pattie.aio.pieces import _PIECE_SECONDS, _Waits


class AsyncPlayout:
    """A voice agent's output for asyncio code. See ``pattie.Playout``.

    push() and interrupt() never wait, so they're plain calls."""

    def __init__(self, playout: Playout) -> None:
        self._playout = playout
        self._waits = _Waits()

    def push(self, data: Buffer, reply: str | None = None) -> int:
        return self._playout.push(data, reply)

    def interrupt(self) -> Interrupted:
        return self._playout.interrupt()

    @property
    def queued_frames(self) -> int:
        return self._playout.queued_frames

    async def wait_idle(self) -> None:
        """Waits until everything pushed has been heard."""
        while not await self._waits.run(self._playout.wait_idle, _PIECE_SECONDS):
            pass

    def close(self) -> None:
        self._playout.close()
        self._waits.shut_down()

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self,
        kind: type[BaseException] | None,
        value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()
