"""A Device's Stats."""

from typing import final

@final
class Stats:
    """Audio a Device lost so far, and how much is waiting now, in frames at the
    Device's rate."""

    @property
    def gap_frames(self) -> int: ...
    @property
    def overflow_frames(self) -> int: ...
    @property
    def buffered_frames(self) -> int: ...
