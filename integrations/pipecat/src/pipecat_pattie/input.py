"""PattieInputTransport: what the meeting plays to the Speaker, as frames."""

import asyncio

import pattie
from pipecat.frames.frames import (
    CancelFrame,
    EndFrame,
    InputAudioRawFrame,
    StartFrame,
)
from pipecat.processors.frame_processor import FrameProcessorSetup
from pipecat.transports.base_input import BaseInputTransport

from pipecat_pattie.devices import _Devices
from pipecat_pattie.params import PattieTransportParams

# The input side's frame length, and how long it waits for audio before
# sending silence in its place: 20 ms, as a microphone's packets are.
_FRAME_SECONDS = 0.02


class PattieInputTransport(BaseInputTransport):
    """What the meeting app plays to the bot's Speaker, into the pipeline."""

    _params: PattieTransportParams

    def __init__(self, devices: _Devices, params: PattieTransportParams) -> None:
        super().__init__(params)
        self._devices = devices
        self._converter: pattie.Converter | None = None
        self._reader: asyncio.Task[None] | None = None

    async def setup(self, setup: FrameProcessorSetup) -> None:
        await super().setup(setup)
        self._devices.open()
        speaker = self._devices.speaker
        assert speaker is not None
        # What read() returns: float at the Speaker's own rate and channels.
        played = pattie.AudioFormat(speaker.rate, "f32", channels=speaker.channels)
        self._converter = pattie.Converter(
            played,
            pattie.AudioFormat(self.sample_rate, "pcm16", channels=self._params.audio_in_channels),
        )

    async def start(self, frame: StartFrame) -> None:
        await super().start(frame)
        if self._reader is None:
            self._reader = self.create_task(self._read_speaker(), name="pattie-speaker")
        await self.set_transport_ready(frame)

    async def stop(self, frame: EndFrame) -> None:
        await super().stop(frame)
        await self._stop_reader()

    async def cancel(self, frame: CancelFrame) -> None:
        await super().cancel(frame)
        await self._stop_reader()

    async def cleanup(self) -> None:
        try:
            await super().cleanup()
            await self._stop_reader()
        finally:
            self._devices.close_speaker()

    async def _stop_reader(self) -> None:
        if self._reader is not None:
            await self.cancel_task(self._reader)
            self._reader = None

    async def _read_speaker(self) -> None:
        """Pushes 20 ms frames of what's played, or of silence while nothing
        is. A wait ends as soon as the Speaker has audio (Pattie wakes on the
        system's pass); silence is paced by the clock, not by how long a wait
        took (Windows rounds short waits up to its 15.6 ms timer tick), so the
        pipeline always gets real time's worth."""
        speaker, converter = self._devices.speaker, self._converter
        assert speaker is not None
        assert converter is not None
        loop = self.get_event_loop()
        frame_bytes = round(self.sample_rate * _FRAME_SECONDS) * 2 * self._params.audio_in_channels
        pending = b""
        silence_due = loop.time() + _FRAME_SECONDS  # when quiet first counts as silence
        while not speaker.closed:
            played = await asyncio.to_thread(speaker.read, _FRAME_SECONDS)
            now = loop.time()
            if played:
                pending += converter.convert(played)
                silence_due = now + _FRAME_SECONDS
            elif not pending:
                while silence_due <= now:
                    pending += bytes(frame_bytes)
                    silence_due += _FRAME_SECONDS
            while len(pending) >= frame_bytes:
                audio, pending = pending[:frame_bytes], pending[frame_bytes:]
                await self.push_audio_frame(
                    InputAudioRawFrame(
                        audio=audio,
                        sample_rate=self.sample_rate,
                        num_channels=self._params.audio_in_channels,
                    )
                )
