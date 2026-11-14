"""A Pipecat transport over Viaduct: the bot gets its own virtual microphone and
speaker, which meeting apps (Zoom, Teams, Meet) list as ordinary devices.

- ``output()``: what the pipeline says goes out through the Microphone, at the
  pace a listener hears it. Pipecat's own output queue stays in charge, so its
  speaking events and interruptions work as with any transport; the device
  holds only Viaduct's lead (~20 ms) beyond the chunk being written.
- ``input()``: what the meeting app plays to the Speaker comes into the
  pipeline as 20 ms frames, with silence filled in while nothing plays, so
  voice activity detection always sees the end of a turn.

Viaduct converts between the pipeline's format (16-bit PCM at its rates) and
the Devices' own (48 kHz mono float by default).
"""

import asyncio
from concurrent.futures import ThreadPoolExecutor

import viaduct
from pipecat.frames.frames import (
    CancelFrame,
    EndFrame,
    InputAudioRawFrame,
    OutputAudioRawFrame,
    StartFrame,
)
from pipecat.processors.frame_processor import FrameProcessor, FrameProcessorSetup
from pipecat.transports.base_input import BaseInputTransport
from pipecat.transports.base_output import BaseOutputTransport
from pipecat.transports.base_transport import BaseTransport, TransportParams


class ViaductTransportParams(TransportParams):
    """Pipecat's transport parameters, plus the Devices'.

    Parameters:
        mic_name: The bot's microphone, as meeting apps list it.
        speaker_name: The bot's speaker, as meeting apps list it.
        device_rate: The Devices' sample rate: 44,100, 48,000 or 96,000.
        device_channels: The Devices' channels, 1 to 8. The pipeline's own
            format is converted to and from it.
        strict: Raise ``viaduct.AudioLost`` when audio is lost, instead of
            logging a warning.
    """

    mic_name: str = "Agent Mic"
    speaker_name: str = "Agent Speaker"
    device_rate: int = 48_000
    device_channels: int = 1
    strict: bool = False


class _Devices:
    """The transport's Devices: created with it, so they can be picked in a
    meeting app before the bot starts; closed when the pipeline ends; created
    again (same name and format, so the same endpoint) if it runs again."""

    def __init__(self, params: ViaductTransportParams, backend: object) -> None:
        self._params = params
        self._backend = backend
        self.microphone: viaduct.Microphone | None = None
        self.speaker: viaduct.Speaker | None = None
        self.open()

    def open(self) -> None:
        params = self._params
        if params.audio_out_enabled and (self.microphone is None or self.microphone.closed):
            self.microphone = viaduct.Microphone(
                params.mic_name,
                rate=params.device_rate,
                channels=params.device_channels,
                strict=params.strict,
                _backend=self._backend,
            )
        if params.audio_in_enabled and (self.speaker is None or self.speaker.closed):
            self.speaker = viaduct.Speaker(
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


# The input side's frame length, and how long it waits for audio before
# sending silence in its place: 20 ms, as a microphone's packets are.
_FRAME_SECONDS = 0.02


class ViaductInputTransport(BaseInputTransport):
    """What the meeting app plays to the bot's Speaker, into the pipeline."""

    _params: ViaductTransportParams

    def __init__(self, devices: _Devices, params: ViaductTransportParams) -> None:
        super().__init__(params)
        self._devices = devices
        self._converter: viaduct.Converter | None = None
        self._reader: asyncio.Task[None] | None = None

    async def setup(self, setup: FrameProcessorSetup) -> None:
        await super().setup(setup)
        self._devices.open()
        speaker = self._devices.speaker
        assert speaker is not None
        # What read() returns: float at the Speaker's own rate and channels.
        played = viaduct.Format(speaker.rate, "f32", channels=speaker.channels)
        self._converter = viaduct.Converter(
            played,
            viaduct.Format(self.sample_rate, "pcm16", channels=self._params.audio_in_channels),
        )

    async def start(self, frame: StartFrame) -> None:
        await super().start(frame)
        if self._reader is None:
            self._reader = self.create_task(self._read_speaker(), name="viaduct-speaker")
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
        is. A wait ends as soon as the Speaker has audio (Viaduct wakes on the
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


class ViaductOutputTransport(BaseOutputTransport):
    """The pipeline's speech, out through the bot's Microphone."""

    _params: ViaductTransportParams

    def __init__(self, devices: _Devices, params: ViaductTransportParams) -> None:
        super().__init__(params)
        self._devices = devices
        self._converter: viaduct.Converter | None = None
        self._executor: ThreadPoolExecutor | None = None

    async def setup(self, setup: FrameProcessorSetup) -> None:
        await super().setup(setup)
        self._devices.open()
        # One writer at a time, in order: the paced write blocks a thread. Made
        # per run, as the Devices are, so the transport can run again.
        self._executor = ThreadPoolExecutor(max_workers=1, thread_name_prefix="viaduct-mic")
        microphone = self._devices.microphone
        assert microphone is not None
        self._converter = viaduct.Converter(
            viaduct.Format(self.sample_rate, "pcm16", channels=self._params.audio_out_channels),
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


class ViaductTransport(BaseTransport):
    """A bot's own virtual microphone (``output()``) and speaker (``input()``).

    ``_backend`` is for tests only (``viaduct._testing.TestBackend``).
    """

    def __init__(
        self,
        params: ViaductTransportParams | None = None,
        *,
        _backend: object = None,
    ) -> None:
        super().__init__()
        self._params = params or ViaductTransportParams(
            audio_in_enabled=True, audio_out_enabled=True
        )
        self._devices = _Devices(self._params, _backend)
        self._input: ViaductInputTransport | None = None
        self._output: ViaductOutputTransport | None = None

    @property
    def microphone(self) -> viaduct.Microphone | None:
        """The bot's microphone, or None without ``audio_out_enabled``."""
        return self._devices.microphone

    @property
    def speaker(self) -> viaduct.Speaker | None:
        """The bot's speaker, or None without ``audio_in_enabled``."""
        return self._devices.speaker

    def input(self) -> FrameProcessor:
        if self._input is None:
            self._input = ViaductInputTransport(self._devices, self._params)
        return self._input

    def output(self) -> FrameProcessor:
        if self._output is None:
            self._output = ViaductOutputTransport(self._devices, self._params)
        return self._output


__all__ = [
    "ViaductInputTransport",
    "ViaductOutputTransport",
    "ViaductTransport",
    "ViaductTransportParams",
]
