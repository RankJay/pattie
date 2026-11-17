"""A Speaker."""

from collections.abc import Iterator
from types import TracebackType
from typing import Self, final

from .clock_point import ClockPoint
from .format import AudioFormat
from .stats import Stats

@final
class Speaker:
    """A virtual speaker: audio other apps play to it is what the app reads."""

    def __new__(
        cls,
        name: str,
        *,
        rate: int = 48_000,
        channels: int = 1,
        format: AudioFormat | None = None,  # noqa: A002
        _backend: object = None,
    ) -> Self: ...
    @property
    def id(self) -> int: ...
    @property
    def rate(self) -> int: ...
    @property
    def channels(self) -> int: ...
    @property
    def closed(self) -> bool: ...
    def read(self, timeout: float | None = 0.0) -> bytes: ...
    def read_timed(self, timeout: float | None = 0.0) -> tuple[bytes, float | None]: ...
    @property
    def clock(self) -> ClockPoint | None: ...
    def __iter__(self) -> Iterator[bytes]: ...
    def __next__(self) -> bytes: ...
    @property
    def stats(self) -> Stats: ...
    def close(self) -> None: ...
    def __enter__(self) -> Self: ...
    def __exit__(
        self,
        kind: type[BaseException] | None,
        value: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool: ...
