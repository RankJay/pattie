"""The stable_id function."""

from typing import Literal

def stable_id(
    kind: Literal["microphone", "speaker"], name: str, rate: int = 48_000, channels: int = 1
) -> str: ...
