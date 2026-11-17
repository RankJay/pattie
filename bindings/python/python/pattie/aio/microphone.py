"""AsyncMicrophone: a Microphone whose writes and waits are awaitable."""

import asyncio
from collections.abc import Buffer
from types import TracebackType
from typing import Self

from pattie._native import (
    AudioFormat,
    ClockPoint,
    Converter,
    LiveWriter,
    Microphone,
    Stats,
)
from pattie.aio.pieces import _PIECE_SECONDS
from pattie.aio.playout import AsyncPlayout

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
        format: AudioFormat | None = None,  # noqa: A002
        strict: bool = False,
        _backend: object = None,
    ) -> None:
        # The microphone below takes the Device's own format; this class converts
        # first. Then a write cut short by its 100 ms wait says exactly which
        # bytes went in, and the next wait carries on from there.
        device = AudioFormat(rate, "f32", channels=channels)
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
    def format(self) -> AudioFormat:
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

    def playout(self, format: AudioFormat | None = None) -> AsyncPlayout:  # noqa: A002
        """The microphone as a voice agent's output. See ``Microphone.playout``."""
        return AsyncPlayout(self._mic.playout(format or self._format))

    def live_writer(self, format: AudioFormat | None = None) -> LiveWriter:  # noqa: A002
        """The microphone as a live source's input, drift-compensated. Its push()
        never waits, so the sync ``LiveWriter`` serves asyncio code as it is. See
        ``Microphone.live_writer``."""
        return self._mic.live_writer(format or self._format)

    @property
    def clock(self) -> ClockPoint | None:
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
