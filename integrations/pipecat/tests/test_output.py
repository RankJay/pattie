"""The output side: what the pipeline says reaches the meeting through the bot's
microphone, at the pace a listener hears it."""

import time
from typing import Any

from conftest import SystemPull, pcm16
from pattie import _testing
from pipecat.frames.frames import InterruptionFrame, OutputAudioRawFrame
from pipecat.tests.utils import SleepFrame, run_test

from pipecat_pattie import PattieTransport, PattieTransportParams


def speaking(backend: _testing.FakeBackend, **params: Any) -> PattieTransport:
    """A transport with only its microphone, the pipeline at 24 kHz."""
    return PattieTransport(
        PattieTransportParams(
            audio_out_enabled=True,
            audio_out_sample_rate=24_000,
            audio_out_end_silence_secs=0,
            **params,
        ),
        _backend=backend,
    )


def speech(seconds: float, value: float = 0.25) -> OutputAudioRawFrame:
    return OutputAudioRawFrame(
        audio=pcm16(value, round(24_000 * seconds)), sample_rate=24_000, num_channels=1
    )


async def test_speech_reaches_the_meeting_converted_to_the_microphones_format(
    backend: _testing.FakeBackend,
) -> None:
    transport = speaking(backend)
    assert transport.microphone is not None
    system = SystemPull(backend, transport.microphone.id)

    await run_test(transport.output(), frames_to_send=[speech(0.2), SleepFrame(sleep=0.3)])
    taken = system.stop()

    # 200 ms at the microphone's 48 kHz, give or take the resampler's edges.
    loud = [s for s in taken if abs(s) > 0.2]
    assert abs(len(loud) - 9_600) < 200, len(loud)
    assert all(abs(s - 0.25) < 0.01 for s in loud[100:-100])


async def test_writes_take_as_long_as_the_speech_as_pipecat_expects(
    backend: _testing.FakeBackend,
) -> None:
    transport = speaking(backend)
    assert transport.microphone is not None
    system = SystemPull(backend, transport.microphone.id)

    started = time.monotonic()
    await run_test(transport.output(), frames_to_send=[speech(1.0)])
    elapsed = time.monotonic() - started
    system.stop()

    # Pipecat's speaking events and interruptions assume a write lasts as long
    # as its audio; a queue that took it all at once would end in milliseconds.
    assert elapsed > 0.9, elapsed


async def test_an_interruption_stops_the_speech_within_a_chunk_and_the_lead(
    backend: _testing.FakeBackend,
) -> None:
    transport = speaking(backend)
    assert transport.microphone is not None
    system = SystemPull(backend, transport.microphone.id)

    await run_test(
        transport.output(),
        frames_to_send=[
            speech(3.0),
            SleepFrame(sleep=0.3),
            InterruptionFrame(),
            SleepFrame(sleep=0.5),
        ],
    )
    taken = system.stop()

    # About 0.3 s played before the interruption, then at most one 40 ms chunk
    # and the device's ~20 ms lead: nowhere near the 3 s that was sent.
    heard = sum(1 for s in taken if abs(s) > 0.2) / 48_000
    assert 0.2 < heard < 0.5, heard
