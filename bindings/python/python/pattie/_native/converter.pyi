"""The Converter."""

from collections.abc import Buffer
from typing import Self, final

from .format import AudioFormat

@final
class Converter:
    """Converts audio between two Formats, keeping its state between calls."""

    def __new__(cls, source: AudioFormat, target: AudioFormat) -> Self: ...
    def convert(self, data: Buffer) -> bytes: ...
