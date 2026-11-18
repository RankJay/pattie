"""A LiveWriter and its stats."""

from collections.abc import Buffer
from types import TracebackType
from typing import Self, final

@final
class LiveStats:
    """How a ``LiveWriter`` is doing."""

    @property
    def queued_frames(self) -> int: ...
    @property
    def correction_ppm(self) -> float: ...
    @property
    def starved_frames(self) -> int: ...
    @property
    def dropped_frames(self) -> int: ...

@final
class LiveWriter:
    """A live source's input, drift-compensated: see ``Microphone.live_writer()``."""

    def push(self, data: Buffer) -> int: ...
    @property
    def stats(self) -> LiveStats: ...
    def close(self) -> None: ...
    def __enter__(self) -> Self: ...
    def __exit__(
        self,
        kind: type[BaseException] | None,
        value: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool: ...
