"""Audio through the public API: what a Microphone writes reaches the system. A
Speaker is in test_speaker_read.py. The rules themselves (pacing, conversion) are
tested in Rust; these tests check that Python reaches them correctly."""

import array
import logging
import threading

import pytest
from system_side import as_floats, floats, pull_in_background

import pattie
from pattie import _testing


def test_written_audio_reaches_the_system_unchanged(backend: _testing.FakeBackend) -> None:
    samples = [i / 1000 for i in range(480)]

    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        assert mic.write(floats(samples)) == 480
        taken = backend.system_read(mic.id, 480)

    assert taken is not None
    assert as_floats(taken) == pytest.approx(samples)


def test_write_takes_any_bytes_like_object(backend: _testing.FakeBackend) -> None:
    samples = array.array("f", [0.25] * 100)

    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        assert mic.write(samples) == 100  # an array of floats, not bytes
        assert mic.write(memoryview(bytearray(floats([0.5] * 100)))) == 100


def test_write_refuses_text(backend: _testing.FakeBackend) -> None:
    with (
        pattie.Microphone("Agent Mic", _backend=backend) as mic,
        pytest.raises(TypeError, match="bytes-like"),
    ):
        mic.write("hello")  # ty: ignore[invalid-argument-type]


def test_write_waits_for_the_system_and_drops_nothing(backend: _testing.FakeBackend) -> None:
    second = [((i % 100) - 50) / 100 for i in range(48_000)]
    done = threading.Event()

    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        taken = pull_in_background(backend, mic.id, done)
        # A whole second at once: far more than the Ring holds, so this only
        # finishes if write() waits for the system and lets the puller run.
        assert mic.write(floats(second), timeout=10) == 48_000
        assert mic.wait_for_playout(timeout=10)
        done.set()
        stats = mic.stats

    # The system took silence until the first write; after that, all of it.
    start = next(i for i, x in enumerate(taken) if x != 0)
    assert taken[start : start + len(second)] == pytest.approx(second)
    assert stats.overflow_frames == 0


def test_write_stops_at_its_timeout_with_only_the_lead_written(
    backend: _testing.FakeBackend,
) -> None:
    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        written = mic.write(floats([0.1] * 48_000), timeout=0.05)

    assert written == backend.lead_frames


def test_played_ms_counts_what_the_system_took(backend: _testing.FakeBackend) -> None:
    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        assert mic.played_ms == 0
        mic.write(floats([0.1] * 480))
        backend.system_read(mic.id, 480)

        assert mic.played_frames == 480
        assert mic.played_ms == 10
        assert mic.wait_for_playout(timeout=0)


def test_the_format_argument_converts_what_write_takes(backend: _testing.FakeBackend) -> None:
    pcm16 = pattie.AudioFormat(48_000, "pcm16")
    half = array.array("h", [16_384] * 480).tobytes()

    with pattie.Microphone("Agent Mic", format=pcm16, _backend=backend) as mic:
        assert mic.format == pcm16
        mic.write(half)
        taken = backend.system_read(mic.id, 480)

    assert taken is not None
    assert as_floats(taken) == pytest.approx([0.5] * 480)


def test_write_nowait_drops_what_does_not_fit_and_logs_it(
    backend: _testing.FakeBackend, caplog: pytest.LogCaptureFixture
) -> None:
    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        with caplog.at_level(logging.WARNING, logger="pattie"):
            written = mic.write_nowait(floats([0.1] * 10_000))

        assert written < 10_000
        assert mic.stats.overflow_frames == 10_000 - written
    assert "Overflow" in caplog.text


def test_strict_raises_audio_lost_instead_of_logging(backend: _testing.FakeBackend) -> None:
    with (
        pattie.Microphone("Agent Mic", strict=True, _backend=backend) as mic,
        pytest.raises(pattie.AudioLost, match="Overflow"),
    ):
        mic.write_nowait(floats([0.1] * 10_000))


def test_a_microphone_has_one_writer(backend: _testing.FakeBackend) -> None:
    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        mic.write(floats([0.1] * 10))
        with pytest.raises(pattie.AlreadyTaken, match="one writer"):
            mic.playout()


def test_a_write_cut_mid_sample_carries_the_rest_to_the_next(
    backend: _testing.FakeBackend,
) -> None:
    samples = [i / 100 for i in range(10)]
    data = floats(samples)

    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        # 5 bytes is one sample and a quarter: the quarter waits for the rest.
        first = mic.write_nowait(data[:5])
        second = mic.write_nowait(data[5:])
        taken = backend.system_read(mic.id, 10)

    assert (first, second) == (1, 9)
    assert taken is not None
    assert as_floats(taken) == pytest.approx(samples)
