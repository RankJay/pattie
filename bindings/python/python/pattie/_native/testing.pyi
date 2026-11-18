"""FakeBackend and sdk_error."""

from collections.abc import Buffer
from typing import Self, final

from .errors import Error

# Only in the project's own test builds (see pattie._testing).
@final
class FakeBackend:
    def __new__(cls) -> Self: ...
    @property
    def lead_frames(self) -> int: ...
    def system_read(self, device_id: int, count: int) -> bytes | None: ...
    def system_write(self, device_id: int, data: Buffer) -> bool: ...

def sdk_error(name: str) -> Error: ...
