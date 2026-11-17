"""The input side: what the meeting app plays to the bot's speaker reaches the
pipeline as 20 ms frames, with silence while nothing plays."""

import array
import asyncio

from conftest import f32
from pattie import _testing
from pipecat.frames.frames import Frame, InputAudioRawFrame
from pipecat.tests.utils import SleepFrame, run_test

from pipecat_pattie import PattieTransport, PattieTransportParams


def listening(backend: _testing.FakeBackend) -> PattieTransport:
    """A transport with only its speaker, the pipeline at 16 kHz."""
    return PattieTransport(
        PattieTransportParams(audio_in_enabled=True, audio_in_sample_rate=16_000),
        _backend=backend,
    )


def audio_frames(frames: object) -> list[InputAudioRawFrame]:
    assert isinstance(frames, list | tuple)
    return [f for f in frames if isinstance(f, InputAudioRawFrame)]


def samples(frames: list[InputAudioRawFrame]) -> list[int]:
    return array.array("h", b"".join(f.audio for f in frames)).tolist()


async def test_audio_played_to_the_speaker_reaches_the_pipeline_as_20_ms_frames(
    backend: _testing.FakeBackend,
) -> None:
    transport = listening(backend)
    assert transport.speaker is not None
    speaker_id = transport.speaker.id

    async def meeting_app_plays() -> None:
        # 200 ms, in the 10 ms packets the system plays in.
        await asyncio.sleep(0.3)
        for _ in range(20):
            assert backend.system_write(speaker_id, f32([0.5] * 480))
            await asyncio.sleep(0.01)

    results = await asyncio.gather(
        run_test(transport.input(), frames_to_send=[SleepFrame(sleep=0.8)]),
        meeting_app_plays(),
    )
    down: list[Frame] = list(results[0][0])
    frames = audio_frames(down)

    # 20 ms of 16-bit mono at 16 kHz is 640 bytes.
    assert frames, "no audio reached the pipeline"
    assert all((len(f.audio), f.sample_rate, f.num_channels) == (640, 16_000, 1) for f in frames)
    # The 200 ms played, once: 3,200 samples at 16 kHz, give or take the edges.
    loud = [s for s in samples(frames) if s > 15_000]
    assert abs(len(loud) - 3_200) < 100, len(loud)


async def test_while_nothing_plays_the_pipeline_hears_silence(
    backend: _testing.FakeBackend,
) -> None:
    transport = listening(backend)

    down, _ = await run_test(transport.input(), frames_to_send=[SleepFrame(sleep=0.5)])
    frames = audio_frames(down)

    # A real microphone keeps sending: voice detection needs the quiet after a
    # turn to know it ended. About 0.5 s of 20 ms frames, all silent.
    seconds = len(frames) * 0.02
    assert 0.3 < seconds < 0.7, seconds
    assert not any(samples(frames))
