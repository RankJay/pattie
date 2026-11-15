"""A live source's writer (drift compensation, ADR 0022) and the device clock's
timestamps, through the public API. The rules are tested in Rust; these tests
check that Python reaches them, and that times come out on time.perf_counter()."""

import array
import threading
import time

import pytest

import pattie
from pattie import _testing

TEN_MS = 0.010
SLACK = 0.002  # the clock's conversion, and a busy test machine


def floats(values: list[float]) -> bytes:
    return array.array("f", values).tobytes()


def as_floats(data: bytes) -> list[float]:
    return array.array("f", data).tolist()


def pull_for(backend: _testing.TestBackend, device_id: int, seconds: float) -> list[float]:
    """Plays the system's part for `seconds`: takes 10 ms of audio every 10 ms."""
    taken: list[float] = []
    until = time.monotonic() + seconds
    while time.monotonic() < until:
        got = backend.driver_read(device_id, 480)
        if got is not None:
            taken.extend(as_floats(got))
        time.sleep(TEN_MS)
    return taken


def test_a_live_writers_audio_reaches_the_system_once_its_buffer_fills(
    backend: _testing.TestBackend,
) -> None:
    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        live = mic.live_writer()
        assert live.push(floats([0.5] * 9_600)) == 9_600  # 200 ms from the source

        taken = pull_for(backend, mic.id, 0.4)

        assert sum(1 for s in taken if abs(s - 0.5) < 1e-3) >= 4_800
        stats = live.stats
        assert (stats.dropped_frames, isinstance(stats.correction_ppm, float)) == (0, True)


def test_a_live_writer_converts_its_format(backend: _testing.TestBackend) -> None:
    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        live = mic.live_writer(pattie.Format(24_000, "pcm16"))

        # 240 samples at 24 kHz: 480 frames at 48 kHz.
        assert live.push(array.array("h", [1_000] * 240).tobytes()) == 480


def test_a_live_writer_past_one_second_drops_its_oldest_audio(
    backend: _testing.TestBackend,
) -> None:
    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        live = mic.live_writer()

        live.push(floats([0.1] * 50_000))

        assert live.stats.dropped_frames > 0
        assert live.stats.queued_frames < 48_000


def test_a_live_writer_holds_the_writer_and_closing_the_mic_ends_it(
    backend: _testing.TestBackend,
) -> None:
    mic = pattie.Microphone("Agent Mic", _backend=backend)
    live = mic.live_writer()
    with pytest.raises(pattie.AlreadyTaken):
        mic.write(floats([0.1] * 10))

    mic.close()

    assert backend.driver_read(mic.id, 480) is None, "the Device is still live"
    with pytest.raises(ValueError, match="closed"):
        live.push(floats([0.1] * 10))


def test_a_microphones_clock_says_how_far_and_when_the_system_took(
    backend: _testing.TestBackend,
) -> None:
    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        mic.write_nowait(floats([0.5] * 960))
        before_any = (mic.clock, mic.next_play_time)

        start = time.perf_counter()
        backend.driver_read(mic.id, 480)
        end = time.perf_counter()
        clock, next_at = mic.clock, mic.next_play_time

    assert before_any == (None, None)
    assert clock is not None
    assert clock.frames == 480
    assert start - SLACK <= clock.at <= end + SLACK
    # 480 frames still ahead of the next one written: 10 ms after the pass.
    assert next_at is not None
    assert start + TEN_MS - SLACK <= next_at <= end + TEN_MS + SLACK


def test_read_timed_says_when_the_first_frame_read_was_played(
    backend: _testing.TestBackend,
) -> None:
    with pattie.Speaker("Agent Speaker", _backend=backend) as speaker:
        assert speaker.read_timed(timeout=0) == (b"", None)

        start = time.perf_counter()
        backend.driver_write(speaker.id, floats([0.25] * 480))
        end = time.perf_counter()
        data, first_at = speaker.read_timed(timeout=1)

        clock = speaker.clock

    assert as_floats(data) == pytest.approx([0.25] * 480)
    # One 10 ms packet, played by a pass that ended between start and end.
    assert first_at is not None
    assert start - TEN_MS - SLACK <= first_at <= end - TEN_MS + SLACK
    assert clock is not None
    assert clock.frames == 480


def test_a_read_timed_from_another_thread_wakes_when_audio_is_played(
    backend: _testing.TestBackend,
) -> None:
    with pattie.Speaker("Agent Speaker", _backend=backend) as speaker:
        result: list[tuple[bytes, float | None]] = []
        reader = threading.Thread(target=lambda: result.append(speaker.read_timed(timeout=5)))
        reader.start()
        time.sleep(0.05)

        backend.driver_write(speaker.id, floats([0.5] * 480))
        reader.join(timeout=2)

    assert not reader.is_alive()
    assert len(result[0][0]) == 480 * 4
