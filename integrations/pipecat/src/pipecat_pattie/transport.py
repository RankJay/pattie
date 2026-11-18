"""A Pipecat transport over Pattie: the bot gets its own virtual microphone and
speaker, which meeting apps (Zoom, Teams, Meet) list as ordinary devices.

- ``output()``: what the pipeline says goes out through the Microphone, at the
  pace a listener hears it. Pipecat's own output queue stays in charge, so its
  speaking events and interruptions work as with any transport; the device
  holds only Pattie's lead (~20 ms) beyond the chunk being written.
- ``input()``: what the meeting app plays to the Speaker comes into the
  pipeline as 20 ms frames, with silence filled in while nothing plays, so
  voice activity detection always sees the end of a turn.

Pattie converts between the pipeline's format (16-bit PCM at its rates) and
the Devices' own (48 kHz mono float by default).
"""

import pattie
from pipecat.processors.frame_processor import FrameProcessor
from pipecat.transports.base_transport import BaseTransport

from pipecat_pattie.devices import _Devices
from pipecat_pattie.input import PattieInputTransport
from pipecat_pattie.output import PattieOutputTransport
from pipecat_pattie.params import PattieTransportParams


class PattieTransport(BaseTransport):
    """A bot's own virtual microphone (``output()``) and speaker (``input()``).

    ``_backend`` is for tests only (``pattie._testing.FakeBackend``).
    """

    def __init__(
        self,
        params: PattieTransportParams | None = None,
        *,
        _backend: object = None,
    ) -> None:
        super().__init__()
        self._params = params or PattieTransportParams(
            audio_in_enabled=True, audio_out_enabled=True
        )
        self._devices = _Devices(self._params, _backend)
        self._input: PattieInputTransport | None = None
        self._output: PattieOutputTransport | None = None

    @property
    def microphone(self) -> pattie.Microphone | None:
        """The bot's microphone, or None without ``audio_out_enabled``."""
        return self._devices.microphone

    @property
    def speaker(self) -> pattie.Speaker | None:
        """The bot's speaker, or None without ``audio_in_enabled``."""
        return self._devices.speaker

    def input(self) -> FrameProcessor:
        if self._input is None:
            self._input = PattieInputTransport(self._devices, self._params)
        return self._input

    def output(self) -> FrameProcessor:
        if self._output is None:
            self._output = PattieOutputTransport(self._devices, self._params)
        return self._output


__all__ = [
    "PattieInputTransport",
    "PattieOutputTransport",
    "PattieTransport",
    "PattieTransportParams",
]
