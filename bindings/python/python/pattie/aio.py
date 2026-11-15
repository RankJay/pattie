"""Pattie for asyncio: the same Devices, with the calls that wait made awaitable.

Waiting happens in a worker thread (``asyncio.to_thread``), so the event loop
keeps running. Each wait in that thread lasts at most 100 ms, so cancelling a call
stops it within about that long, and closing never waits on a stuck call. To give
up after a while, use ``asyncio.timeout``::

    async with AsyncMicrophone("Agent Mic", format=Format(24_000, "pcm16")) as mic:
        await mic.write(audio)
        async with asyncio.timeout(5):
            await mic.wait_for_playout()
"""

import asyncio
from collections.abc import AsyncIterator, Buffer
from types import TracebackType
from typing import Self

from pattie._pattie import (
    Clock,
    Converter,
    Format,
    Interrupted,
    LiveWriter,
    Microphone,
    Playout,
    Speaker,
    Stats,
)

__all__ = ["AsyncMicrophone", "AsyncPlayout", "AsyncSpeaker"]

# The longest one wait in the worker thread lasts: 100 ms.
_PIECE_SECONDS = 0.1

# One float sample, in bytes: a frame of the Microphone below is one per channel.
_SAMPLE_BYTES = 4


class AsyncMicrophone:
    """A virtual microphone for asyncio code. See ``pattie.Microphone``."""

    def __init__(
        self,
        name: str,
        *,
        rate: int = 48_000,
        channels: int = 1,
        format: Format | None = None,  # noqa: A002
        strict: bool = False,
        _backend: object = None,
    ) -> None:
        # The microphone below takes the Device's own format; this class converts
        # first. Then a write cut short by its 100 ms wait says exactly which
        # bytes went in, and the next wait carries on from there.
        device = Format(rate, "f32", channels=channels)
        self._format = format or device
        self._input = Converter(self._format, device)
        self._frame_bytes = _SAMPLE_BYTES * channels
        self._mic = Microphone(
            name, rate=rate, channels=channels, strict=strict, _backend=_backend
        )

    @property
    def id(self) -> int:
        return self._mic.id

    @property
    def format(self) -> Format:
        return self._format

    @property
    def rate(self) -> int:
        return self._mic.rate

    @property
    def channels(self) -> int:
        return self._mic.channels

    @property
    def closed(self) -> bool:
        return self._mic.closed

    @property
    def played_ms(self) -> int:
        return self._mic.played_ms

    @property
    def stats(self) -> Stats:
        return self._mic.stats

    async def write(self, data: Buffer) -> int:
        """Writes ``data`` at the pace listeners take it, without blocking the
        event loop. Returns how many of the microphone's frames went in."""
        samples = memoryview(self._input.convert(data))
        written = 0
        while written * self._frame_bytes < len(samples):
            rest = samples[written * self._frame_bytes :]
            written += await asyncio.to_thread(self._mic.write, rest, _PIECE_SECONDS)
        return written

    def write_nowait(self, data: Buffer) -> int:
        """Writes what fits right now; the rest is dropped (and logged). Never waits."""
        return self._mic.write_nowait(self._input.convert(data))

    async def wait_for_playout(self) -> None:
        """Waits until listeners have taken everything written so far."""
        while not await asyncio.to_thread(self._mic.wait_for_playout, _PIECE_SECONDS):
            pass

    def playout(self, format: Format | None = None) -> AsyncPlayout:  # noqa: A002
        """The microphone as a voice agent's output. See ``Microphone.playout``."""
        return AsyncPlayout(self._mic.playout(format or self._format))

    def live_writer(self, format: Format | None = None) -> LiveWriter:  # noqa: A002
        """The microphone as a live source's input, drift-compensated. Its push()
        never waits, so the sync ``LiveWriter`` serves asyncio code as it is. See
        ``Microphone.live_writer``."""
        return self._mic.live_writer(format or self._format)

    @property
    def clock(self) -> Clock | None:
        """See ``Microphone.clock``. Never waits."""
        return self._mic.clock

    @property
    def next_play_time(self) -> float | None:
        """See ``Microphone.next_play_time``. Never waits."""
        return self._mic.next_play_time

    async def aclose(self) -> None:
        """Removes the microphone. Waits for a write in progress to finish its piece."""
        await asyncio.to_thread(self._mic.close)

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self,
        kind: type[BaseException] | None,
        value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        await self.aclose()


class AsyncPlayout:
    """A voice agent's output for asyncio code. See ``pattie.Playout``.

    push() and interrupt() never wait, so they're plain calls."""

    def __init__(self, playout: Playout) -> None:
        self._playout = playout

    def push(self, data: Buffer) -> int:
        return self._playout.push(data)

    def interrupt(self) -> Interrupted:
        return self._playout.interrupt()

    @property
    def queued_frames(self) -> int:
        return self._playout.queued_frames

    async def wait_idle(self) -> None:
        """Waits until everything pushed has been heard."""
        while not await asyncio.to_thread(self._playout.wait_idle, _PIECE_SECONDS):
            pass

    def close(self) -> None:
        self._playout.close()

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(
        self,
        kind: type[BaseException] | None,
        value: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        self.close()


class AsyncSpeaker:
    """A virtual speaker for asyncio code. See ``pattie.Speaker``.

    ``async for chunk in speaker`` yields audio as other apps play it."""

    def __init__(
        self,
        name: str,
        *,
        rate: int = 48_000,
        channels: int = 1,
        format: Format | None = None,  # noqa: A002
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
    def clock(self) -> Clock | None:
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
