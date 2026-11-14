"""Devices at other rates and channel counts (ADR 0023): what Python passes in
reaches the SDK's Device format, and frames are counted as whole frames."""

import array
import asyncio
import time

import pytest

import viaduct
from viaduct import _testing
from viaduct.aio import AsyncMicrophone


def floats(values: list[float]) -> bytes:
    return array.array("f", values).tobytes()


def as_floats(data: bytes) -> list[float]:
    return array.array("f", data).tolist()


def stereo(frames: int) -> list[float]:
    """Interleaved frames: left n / 1000, right -(n / 1000)."""
    return [value for n in range(1, frames + 1) for value in (n / 1000, -n / 1000)]


def test_a_stereo_microphone_says_its_format_and_carries_whole_frames(
    backend: _testing.TestBackend,
) -> None:
    samples = stereo(441)

    with viaduct.Microphone("Stereo Mic", rate=44_100, channels=2, _backend=backend) as mic:
        written = mic.write_nowait(floats(samples))
        taken = backend.driver_read(mic.id, 441)

        assert (mic.rate, mic.channels) == (44_100, 2)
        assert mic.format == viaduct.Format(44_100, "f32", channels=2)

    assert written == 441, "frames"
    assert taken is not None
    assert as_floats(taken) == pytest.approx(samples)


def test_a_paced_write_to_a_stereo_microphone_counts_frames(
    backend: _testing.TestBackend,
) -> None:
    with viaduct.Microphone("Stereo Mic", channels=2, _backend=backend) as mic:
        written = mic.write(floats(stereo(4_800)), timeout=0.05)

    # Only the lead goes in before the timeout: well under 4,800 frames, and a
    # count of frames, not of samples.
    assert 0 < written <= backend.lead_frames


def test_a_write_cut_mid_frame_carries_the_rest_to_the_next(
    backend: _testing.TestBackend,
) -> None:
    samples = stereo(2)

    with viaduct.Microphone("Stereo Mic", channels=2, _backend=backend) as mic:
        # Three samples are a frame and a half: the half waits for the rest.
        first = mic.write_nowait(floats(samples[:3]))
        second = mic.write_nowait(floats(samples[3:]))
        taken = backend.driver_read(mic.id, 2)

    assert (first, second) == (1, 1)
    assert taken is not None
    assert as_floats(taken) == pytest.approx(samples)


def test_mono_written_to_a_stereo_microphone_fills_both_channels(
    backend: _testing.TestBackend,
) -> None:
    with viaduct.Microphone(
        "Stereo Mic", channels=2, format=viaduct.Format(48_000), _backend=backend
    ) as mic:
        written = mic.write_nowait(floats([0.5] * 480))
        taken = backend.driver_read(mic.id, 480)

    assert written == 480
    assert taken is not None
    assert as_floats(taken) == pytest.approx([0.5] * 960)


def test_a_stereo_96_khz_speaker_reads_whole_frames(backend: _testing.TestBackend) -> None:
    samples = stereo(960)

    with viaduct.Speaker("Stereo Speaker", rate=96_000, channels=2, _backend=backend) as speaker:
        assert backend.driver_write(speaker.id, floats(samples))
        data = speaker.read()

        assert (speaker.rate, speaker.channels) == (96_000, 2)

    assert as_floats(data) == pytest.approx(samples)


def test_a_mono_speaker_format_mixes_a_stereo_speaker_down(
    backend: _testing.TestBackend,
) -> None:
    with viaduct.Speaker(
        "Stereo Speaker", channels=2, format=viaduct.Format(48_000), _backend=backend
    ) as speaker:
        backend.driver_write(speaker.id, floats([0.25, 0.75] * 480))
        data = speaker.read()

    assert as_floats(data) == pytest.approx([0.5] * 480)


def test_heard_ms_counts_at_the_microphones_rate(backend: _testing.TestBackend) -> None:
    with (
        viaduct.Microphone("Agent Mic", rate=96_000, _backend=backend) as mic,
        mic.playout() as playout,
    ):
        playout.push(floats([0.5] * 96_000))
        deadline = time.monotonic() + 5
        taken = 0
        while taken < 9_600 and time.monotonic() < deadline:
            got = backend.driver_read(mic.id, 960)
            assert got is not None
            taken += sum(1 for x in as_floats(got) if x != 0)
            time.sleep(0.005)
        cut = playout.interrupt()

    assert cut.heard_frames >= 9_600
    assert cut.heard_ms == cut.heard_frames * 1000 // 96_000


def test_played_ms_counts_at_the_microphones_rate(backend: _testing.TestBackend) -> None:
    with viaduct.Microphone("Agent Mic", rate=44_100, _backend=backend) as mic:
        mic.write_nowait(floats([0.1] * 2_205))
        backend.driver_read(mic.id, 2_205)

        assert mic.played_frames == 2_205
        assert mic.played_ms == 50, "2,205 frames at 44.1 kHz"


@pytest.mark.parametrize(("rate", "channels"), [(24_000, 1), (48_000, 9), (48_000, 0)])
def test_a_device_format_outside_the_three_rates_and_eight_channels_is_refused(
    backend: _testing.TestBackend, rate: int, channels: int
) -> None:
    with pytest.raises(viaduct.UnsupportedFormat, match="kHz"):
        viaduct.Microphone("Agent Mic", rate=rate, channels=channels, _backend=backend)


def test_an_async_stereo_microphone_writes_whole_frames(backend: _testing.TestBackend) -> None:
    async def main() -> None:
        async with AsyncMicrophone("Stereo Mic", rate=44_100, channels=2, _backend=backend) as mic:
            assert mic.format == viaduct.Format(44_100, "f32", channels=2)
            done = asyncio.Event()

            async def pull() -> int:
                taken = 0
                while not done.is_set():
                    got = backend.driver_read(mic.id, 441)
                    taken += len(got or b"") // 8
                    await asyncio.sleep(0.005)
                return taken

            puller = asyncio.create_task(pull())
            assert await mic.write(floats(stereo(4_410))) == 4_410
            async with asyncio.timeout(10):
                await mic.wait_for_playout()
            done.set()
            assert await puller >= 4_410

    asyncio.run(main())
