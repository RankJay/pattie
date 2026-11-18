"""PattieOutputTransport: what the pipeline says, out through the Microphone."""

import asyncio
from concurrent.futures import ThreadPoolExecutor

import pattie
from loguru import logger
from pipecat.frames.frames import (
    OutputAudioRawFrame,
    StartFrame,
)
from pipecat.processors.frame_processor import FrameProcessorSetup
from pipecat.transports.base_output import BaseOutputTransport

from pipecat_pattie.devices import _Devices
from pipecat_pattie.params import PattieTransportParams

# One system pass: the shortest wait that can see whether anything records.
_PASS_SECONDS = 0.012


class PattieOutputTransport(BaseOutputTransport):
    """The pipeline's speech, out through the bot's Microphone."""

    _params: PattieTransportParams

    def __init__(self, devices: _Devices, params: PattieTransportParams) -> None:
        super().__init__(params)
        self._devices = devices
        self._converter: pattie.Converter | None = None
        self._executor: ThreadPoolExecutor | None = None
        # Set while nothing records the microphone, so that's logged once.
        self._unheard = False

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
            # Close first: a write still in progress then returns within its
            # 100 ms piece, so the writer thread can end. Off the event loop, as
            # both wait for that write; on it, Ctrl+C couldn't get through.
            executor, self._executor = self._executor, None

            def close() -> None:
                self._devices.close_microphone()
                if executor is not None:
                    executor.shutdown(wait=True)

            await asyncio.to_thread(close)

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
        await self.get_event_loop().run_in_executor(self._executor, self._write, microphone, audio)
        return True

    def _write(self, microphone: pattie.Microphone, audio: bytes) -> None:
        """On the writer thread: `audio` (the microphone's own float format) at
        the pace listeners take it. While nothing records the microphone (no
        meeting app has it open), nothing takes audio: the chunk then lasts its
        own duration and is dropped, as with a real microphone nobody listens
        to, so Pipecat's clock stays true and a write never waits for good."""
        frame_bytes = 4 * microphone.channels
        frames = len(audio) // frame_bytes
        written = 0
        while written < frames and not microphone.closed:
            played = microphone.played_ms
            # At least one system pass (10 ms), so a short remainder still
            # sees whether anything is taking audio.
            seconds = max((frames - written) / microphone.rate, _PASS_SECONDS)
            written += microphone.write(audio[written * frame_bytes :], timeout=seconds)
            if written < frames and microphone.played_ms == played:
                if not self._unheard:
                    self._unheard = True
                    logger.warning(
                        f'{self}: nothing is recording "{self._params.mic_name}", so what the '
                        "bot says is dropped until a meeting app picks it as its microphone"
                    )
                return
        self._unheard = False
