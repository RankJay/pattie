"""_Devices: opens and closes the Microphone and the Speaker."""

import pattie

from pipecat_pattie.params import PattieTransportParams


class _Devices:
    """The transport's Devices: created with it, so they can be picked in a
    meeting app before the bot starts; closed when the pipeline ends; created
    again (same name and format, so the same endpoint) if it runs again."""

    def __init__(self, params: PattieTransportParams, backend: object) -> None:
        self._params = params
        self._backend = backend
        self.microphone: pattie.Microphone | None = None
        self.speaker: pattie.Speaker | None = None
        self.open()

    def open(self) -> None:
        params = self._params
        if params.audio_out_enabled and (self.microphone is None or self.microphone.closed):
            self.microphone = pattie.Microphone(
                params.mic_name,
                rate=params.device_rate,
                channels=params.device_channels,
                strict=params.strict,
                _backend=self._backend,
            )
        if params.audio_in_enabled and (self.speaker is None or self.speaker.closed):
            self.speaker = pattie.Speaker(
                params.speaker_name,
                rate=params.device_rate,
                channels=params.device_channels,
                _backend=self._backend,
            )

    def close_microphone(self) -> None:
        if self.microphone is not None:
            self.microphone.close()

    def close_speaker(self) -> None:
        if self.speaker is not None:
            self.speaker.close()
