"""An echo bot: it hears the meeting through its own speaker and says it back
through its own microphone a second later. No API keys, no services: a check
that a meeting app and a Pipecat pipeline reach each other through Viaduct.

    uv run examples/echo_bot.py

Then, in Zoom (or Teams, Meet, Discord): pick "Microphone (Agent Mic)" as the
microphone and "Speakers (Agent Speaker)" as the speaker, join a test call,
and talk: you hear yourself a second later. Ctrl+C to stop.

A real bot puts speech-to-text, an LLM and text-to-speech between
`transport.input()` and `transport.output()`, as in Pipecat's own examples.
"""

import asyncio
from collections import deque

from pipecat.frames.frames import Frame, InputAudioRawFrame, OutputAudioRawFrame
from pipecat.pipeline.pipeline import Pipeline
from pipecat.pipeline.runner import WorkerRunner
from pipecat.pipeline.worker import PipelineParams, PipelineWorker
from pipecat.processors.frame_processor import FrameDirection, FrameProcessor

from pipecat_viaduct import ViaductTransport, ViaductTransportParams

RATE = 16_000  # the pipeline's rate both ways; Viaduct converts to the Devices'


class Echo(FrameProcessor):
    """Turns what it hears into what it says, `delay` seconds later."""

    def __init__(self, delay: float) -> None:
        super().__init__()
        self._frames: deque[InputAudioRawFrame] = deque()
        self._delay = delay

    async def process_frame(self, frame: Frame, direction: FrameDirection) -> None:
        await super().process_frame(frame, direction)
        if not isinstance(frame, InputAudioRawFrame):
            await self.push_frame(frame, direction)
            return
        self._frames.append(frame)
        held = sum(len(f.audio) for f in self._frames) / (2 * frame.sample_rate)
        if held > self._delay:
            heard = self._frames.popleft()
            await self.push_frame(
                OutputAudioRawFrame(
                    audio=heard.audio,
                    sample_rate=heard.sample_rate,
                    num_channels=heard.num_channels,
                )
            )


async def main() -> None:
    transport = ViaductTransport(
        ViaductTransportParams(
            audio_in_enabled=True,
            audio_out_enabled=True,
            audio_in_sample_rate=RATE,
            audio_out_sample_rate=RATE,
        )
    )
    print('"Agent Mic" and "Agent Speaker" are ready: pick them in your meeting app.')
    pipeline = Pipeline([transport.input(), Echo(delay=1.0), transport.output()])
    worker = PipelineWorker(
        pipeline,
        params=PipelineParams(audio_in_sample_rate=RATE, audio_out_sample_rate=RATE),
        idle_timeout_secs=None,
    )
    runner = WorkerRunner()
    await runner.add_workers(worker)
    await runner.run()


if __name__ == "__main__":
    asyncio.run(main())
