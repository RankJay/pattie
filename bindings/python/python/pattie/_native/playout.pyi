"""A Playout, and what an interrupt reports."""

from collections.abc import Buffer
from types import TracebackType
from typing import Self, final

@final
class Interrupted:
    """What ``Playout.interrupt()`` cut."""

    @property
    def heard_frames(self) -> int: ...
    @property
    def discarded_frames(self) -> int: ...
    @property
    def heard_ms(self) -> int: ...

@final
class Playout:
    """A voice agent's output: see ``Microphone.playout()``."""

    def push(self, data: Buffer) -> int: ...
    def interrupt(self) -> Interrupted: ...
    def wait_idle(self, timeout: float | None = None) -> bool: ...
    @property
    def queued_frames(self) -> int: ...
    def close(self) -> None: ...
    def __enter__(self) -> Self: ...
    def __exit__(
        self,
        kind: type[BaseException] | None,
        value: BaseException | None,
        traceback: TracebackType | None,
    ) -> bool: ...
