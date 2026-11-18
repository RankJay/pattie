"""One reading of the Device clock."""

from typing import final

@final
class ClockPoint:
    """Where the system's side of a Device had got to: ``frames`` (at its rate) at
    ``at``, a ``time.perf_counter()`` time."""

    @property
    def frames(self) -> int: ...
    @property
    def at(self) -> float: ...
