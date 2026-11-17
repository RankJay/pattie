"""PattieOutputTransport: what the pipeline says, out through the Microphone."""

from concurrent.futures import ThreadPoolExecutor

import pattie
from pipecat.frames.frames import (
    OutputAudioRawFrame,
    StartFrame,
)
from pipecat.processors.frame_processor import FrameProcessorSetup
from pipecat.transports.base_output import BaseOutputTransport

from pipecat_pattie.devices import _Devices
from pipecat_pattie.params import PattieTransportParams


class PattieOutputTransport(BaseOutputTransport):
    """The pipeline's speech, out through the bot's Microphone."""

    _params: PattieTransportParams

    def __init__(self, devices: _Devices, params: PattieTransportParams) -> None:
        super().__init__(params)
        self._devices = devices
        self._converter: pattie.Converter | None = None
        self._executor: ThreadPoolExecutor | None = None

    async def setup(self, setup: FrameProcessorSetup) -> None:
        await super().setup(setup)
        self._devices.open()
        # One writer at a time, in order: the paced write blocks a thread. Made
        # per run, as the Devices are, so the transport can run again.
        self._executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="pattie-mic")
        microphone = self._devices.microphone
        assert microphone is not None
        self._converter = pattie.Converter(
            pattie.AudioFormat(
                self.sample_rate, "pcm16", channels=self._params.audio_out_channels
            ),
            microphone.format,
        )

    async def start(self, frame: StartFrame) -> None:
        await super().start(frame)
        await self.set_transport_ready(frame)

    async def cleanup(self) -> None:
        try:
            await super().cleanup()
        finally:
            if self._executor is not None:
                self._executor.shutdown(wait=True)
                self._executor = None
            self._devices.close_microphone()

    async def write_audio_frame(self, frame: OutputAudioRawFrame) -> bool:
        """Writes the chunk at the pace listeners take it (``Microphone.write``
        blocks until it's all in), so Pipecat's clock stays true."""
        microphone = self._devices.microphone
        if (
            microphone is None
            or microphone.closed
            or self._converter is None
            or self._executor is None
        ):
            return False
        audio = self._converter.convert(frame.audio)
        await self.get_event_loop().run_in_executor(self._executor, microphone.write, audio)
        return True
