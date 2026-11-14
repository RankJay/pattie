"""When the bot's Devices exist, which ones, and what goes wrong out loud."""

import asyncio
from typing import Any

import pytest
import viaduct
from conftest import SystemPull, pcm16
from pipecat.frames.frames import OutputAudioRawFrame
from pipecat.pipeline.pipeline import Pipeline
from pipecat.tests.utils import SleepFrame, run_test
from viaduct import _testing

from pipecat_viaduct import ViaductTransport, ViaductTransportParams


def both(backend: _testing.TestBackend, **params: Any) -> ViaductTransport:
    return ViaductTransport(
        ViaductTransportParams(
            audio_in_enabled=True,
            audio_out_enabled=True,
            audio_out_sample_rate=24_000,
            audio_out_end_silence_secs=0,
            **params,
        ),
        _backend=backend,
    )


async def run_briefly(transport: ViaductTransport) -> None:
    pipeline = Pipeline([transport.input(), transport.output()])
    await run_test(pipeline, frames_to_send=[SleepFrame(sleep=0.2)])


async def test_the_devices_exist_before_the_bot_runs_and_go_when_it_ends(
    backend: _testing.TestBackend,
) -> None:
    transport = both(backend)
    assert transport.microphone is not None
    assert transport.speaker is not None
    mic_id, speaker_id = transport.microphone.id, transport.speaker.id
    # Listed already, so a meeting app can be pointed at them first.
    assert backend.driver_read(mic_id, 1) is not None

    system = SystemPull(backend, mic_id)
    await run_briefly(transport)
    system.stop()

    assert transport.microphone.closed
    assert transport.speaker.closed
    assert backend.driver_read(mic_id, 1) is None
    assert backend.driver_write(speaker_id, b"\0\0\0\0") is False


async def test_running_again_creates_the_devices_again_under_the_same_names(
    backend: _testing.TestBackend,
) -> None:
    transport = both(backend)
    assert transport.microphone is not None
    first = transport.microphone

    await run_briefly(transport)
    # The second run speaks: the new microphone carries it.
    again = None

    async def speak_again() -> list[float]:
        nonlocal again
        pipeline = Pipeline([transport.input(), transport.output()])
        speech = OutputAudioRawFrame(audio=pcm16(0.25, 4_800), sample_rate=24_000, num_channels=1)
        system: SystemPull | None = None

        # The Device is created when the run sets up; pull from it once it exists.
        async def pull_when_ready() -> None:
            nonlocal system
            while transport.microphone is first:  # noqa: ASYNC110 (the transport has no event for it)
                await asyncio.sleep(0.01)
            assert transport.microphone is not None
            system = SystemPull(backend, transport.microphone.id)

        await asyncio.gather(
            run_test(
                pipeline, frames_to_send=[SleepFrame(sleep=0.1), speech, SleepFrame(sleep=0.4)]
            ),
            pull_when_ready(),
        )
        again = transport.microphone
        assert system is not None
        return system.stop()

    taken = await speak_again()

    assert again is not None
    assert again is not first, "a new Device for the new run"
    assert again.closed, "and closed when that run ended"
    assert sum(1 for s in taken if abs(s) > 0.2) > 4_000, "the second run was heard"
    # Same name and format, so the system gives back the same endpoint (ADR 0015).
    assert (again.rate, again.channels) == (first.rate, first.channels)


def test_only_the_enabled_sides_get_a_device(backend: _testing.TestBackend) -> None:
    speaking = ViaductTransport(ViaductTransportParams(audio_out_enabled=True), _backend=backend)
    listening = ViaductTransport(ViaductTransportParams(audio_in_enabled=True), _backend=backend)

    assert speaking.microphone is not None
    assert speaking.speaker is None
    assert listening.speaker is not None
    assert listening.microphone is None


def test_by_default_the_bot_has_both_at_48_khz_mono(backend: _testing.TestBackend) -> None:
    transport = ViaductTransport(_backend=backend)

    assert transport.microphone is not None
    assert transport.speaker is not None
    assert (transport.microphone.rate, transport.microphone.channels) == (48_000, 1)


def test_the_devices_format_is_a_parameter(backend: _testing.TestBackend) -> None:
    transport = both(backend, device_rate=44_100, device_channels=2)

    assert transport.speaker is not None
    assert (transport.speaker.rate, transport.speaker.channels) == (44_100, 2)


def test_a_name_already_in_use_raises_viaducts_own_error(backend: _testing.TestBackend) -> None:
    first = both(backend)

    with pytest.raises(viaduct.DuplicateName, match="pick another name"):
        both(backend)
    assert first.microphone is not None
    assert not first.microphone.closed
