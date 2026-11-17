"""AsyncSpeaker: a Speaker whose reads are awaitable."""

import asyncio
from collections.abc import AsyncIterator
from types import TracebackType
from typing import Self

from pattie._native import (
    AudioFormat,
    ClockPoint,
    Speaker,
    Stats,
)
from pattie.aio.pieces import _PIECE_SECONDS


class AsyncSpeaker:
    """A virtual speaker for asyncio code. See ``pattie.Speaker``.

    ``async for chunk in speaker`` yields audio as other apps play it."""

    def __init__(
        self,
        name: str,
        *,
        rate: int = 48_000,
        channels: int = 1,
        format: AudioFormat | None = None,  # noqa: A002
        _backend: object = None,
    ) -> None:
        self._speaker = Speaker(
            name, rate=rate, channels=channels, format=format, _backend=_backend
        )

    @property
    def id(self) -> int:
        return self._speaker.id

    @property
    def rate(self) -> int:
        return self._speaker.rate

    @property
    def channels(self) -> int:
        return self._speaker.channels

    @property
    def closed(self) -> bool:
        return self._speaker.closed

    @property
    def stats(self) -> Stats:
        return self._speaker.stats

    async def read(self) -> bytes:
        """Waits for other apps to play something, and returns it."""
        while not (chunk := await asyncio.to_thread(self._speaker.read, _PIECE_SECONDS)):
            pass
        return chunk

    async def read_timed(self) -> tuple[bytes, float | None]:
        """Waits for other apps to play something, and returns it with when its
        first frame was played (a ``time.perf_counter()`` time)."""
        while True:
            data, first_at = await asyncio.to_thread(self._speaker.read_timed, _PIECE_SECONDS)
            if data:
                return data, first_at

    @property
    def clock(self) -> ClockPoint | None:
        """See ``Speaker.clock``."""
        return self._speaker.clock

    async def __aiter__(self) -> AsyncIterator[bytes]:
        # Short waits, so the loop notices aclose() and cancellation soon.
        while not self._speaker.closed:
            chunk = await asyncio.to_thread(self._speaker.read, _PIECE_SECONDS)
            if chunk:
                yield chunk

    async def aclose(self) -> None:
        """Removes the speaker."""
        await asyncio.to_thread(self._speaker.close)

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self,
        kind: type[BaseException] | None,
        value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        await self.aclose()
