"""A Microphone."""

from collections.abc import Buffer
from types import TracebackType
from typing import Self, final

from .clock_point import ClockPoint
from .format import AudioFormat
from .live import LiveWriter
from .playout import Playout
from .stats import Stats

@final
class Microphone:
    """A virtual microphone: audio the app writes is what other apps record."""

    def __new__(
        cls,
        name: str,
        *,
        rate: int = 48_000,
        channels: int = 1,
        format: AudioFormat | None = None,  # noqa: A002
        strict: bool = False,
        _backend: object = None,
    ) -> Self: ...
    @property
    def id(self) -> int: ...
    @property
    def format(self) -> AudioFormat: ...
    @property
    def rate(self) -> int: ...
    @property
    def channels(self) -> int: ...
    @property
    def closed(self) -> bool: ...
    def write(self, data: Buffer, timeout: float | None = None) -> int: ...
    def write_nowait(self, data: Buffer) -> int: ...
    def wait_for_playout(self, timeout: float | None = None) -> bool: ...
    @property
    def played_frames(self) -> int: ...
    @property
    def played_ms(self) -> int: ...
    @property
    def stats(self) -> Stats: ...
    def playout(self, format: AudioFormat | None = None) -> Playout: ...  # noqa: A002
    def live_writer(self, format: AudioFormat | None = None) -> LiveWriter: ...  # noqa: A002
    @property
    def clock(self) -> ClockPoint | None: ...
    @property
    def next_play_time(self) -> float | None: ...
    def close(self) -> None: ...
    def __enter__(self) -> Self: ...
    def __exit__(
        self,
        kind: type[BaseException] | None,
        value: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool: ...
