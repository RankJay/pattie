"""AudioFormat and its encodings."""

from typing import ClassVar, Literal, Self, final

type Encoding = Literal["f32", "pcm16", "ulaw"]

@final
class AudioFormat:
    """A stream's sample rate, encoding and channel count."""

    DEVICE: ClassVar[AudioFormat]
    def __new__(cls, rate: int, encoding: Encoding = "f32", channels: int = 1) -> Self: ...
    @property
    def rate(self) -> int: ...
    @property
    def encoding(self) -> Encoding: ...
    @property
    def channels(self) -> int: ...
    @property
    def frame_bytes(self) -> int: ...
