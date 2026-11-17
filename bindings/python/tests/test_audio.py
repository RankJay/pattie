"""Audio through the public API: what a Microphone writes reaches the system, and
what the system plays reaches a Speaker. The rules themselves (pacing, conversion)
are tested in Rust; these tests check that Python reaches them correctly."""

import array
import logging
import threading
import time

import pytest

import pattie
from pattie import _testing


def floats(values: list[float]) -> bytes:
    return array.array("f", values).tobytes()


def as_floats(data: bytes) -> list[float]:
    return array.array("f", data).tolist()


def pull_in_background(
    backend: _testing.FakeBackend, device_id: int, until: threading.Event
) -> list[float]:
    """Plays the system's part: takes 10 ms of audio every 5 ms, like a sound
    card a little faster than real time. Runs in a Python thread, so it only gets
    anywhere while write() has released the GIL."""
    taken: list[float] = []

    def run() -> None:
        while not until.is_set():
            got = backend.system_read(device_id, 480)
            if got is not None:
                taken.extend(as_floats(got))
            time.sleep(0.005)

    threading.Thread(target=run, daemon=True).start()
    return taken


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


def test_the_speaker_reads_what_the_system_played(backend: _testing.FakeBackend) -> None:
    samples = [i / 1000 for i in range(480)]

    with pattie.Speaker("Agent Speaker", _backend=backend) as speaker:
        assert speaker.read() == b""  # nothing played yet
        assert backend.system_write(speaker.id, floats(samples))
        heard = speaker.read()

    assert as_floats(heard) == pytest.approx(samples)


def test_speaker_read_waits_for_audio_up_to_its_timeout(backend: _testing.FakeBackend) -> None:
    with pattie.Speaker("Agent Speaker", _backend=backend) as speaker:
        timer = threading.Timer(0.05, backend.system_write, (speaker.id, floats([0.5] * 480)))
        timer.start()
        heard = speaker.read(timeout=5)
        timer.join()

    assert as_floats(heard)[:1] == pytest.approx([0.5])


def test_the_speakers_format_converts_what_read_returns(backend: _testing.FakeBackend) -> None:
    pcm16 = pattie.AudioFormat(48_000, "pcm16")

    with pattie.Speaker("Agent Speaker", format=pcm16, _backend=backend) as speaker:
        backend.system_write(speaker.id, floats([0.5] * 480))
        heard = array.array("h", speaker.read()).tolist()

    assert heard == [16_384] * 480


def test_iterating_a_speaker_yields_what_was_played(backend: _testing.FakeBackend) -> None:
    with pattie.Speaker("Agent Speaker", _backend=backend) as speaker:
        backend.system_write(speaker.id, floats([0.25] * 480))
        first = next(iter(speaker))

    assert as_floats(first) == pytest.approx([0.25] * 480)


def test_a_playout_plays_what_was_pushed_and_interrupt_cuts_it(
    backend: _testing.FakeBackend,
) -> None:
    with pattie.Microphone("Agent Mic", _backend=backend) as mic, mic.playout() as playout:
        assert playout.push(floats([0.5] * 48_000)) == 48_000  # returns at once

        deadline = time.monotonic() + 5
        taken: list[float] = []
        while len(taken) < 960 and time.monotonic() < deadline:
            got = backend.system_read(mic.id, 480)
            assert got is not None
            taken.extend(x for x in as_floats(got) if x != 0)
            time.sleep(0.005)
        cut = playout.interrupt()

    assert taken[:960] == pytest.approx([0.5] * 960)
    assert cut.heard_frames >= 960
    assert cut.heard_ms == cut.heard_frames * 1000 // 48_000
    assert cut.heard_frames + cut.discarded_frames <= 48_000


def test_a_microphone_has_one_writer(backend: _testing.FakeBackend) -> None:
    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        mic.write(floats([0.1] * 10))
        with pytest.raises(pattie.AlreadyTaken, match="one writer"):
            mic.playout()


def test_wait_idle_is_true_once_everything_pushed_was_heard(
    backend: _testing.FakeBackend,
) -> None:
    done = threading.Event()
    with pattie.Microphone("Agent Mic", _backend=backend) as mic, mic.playout() as playout:
        pull_in_background(backend, mic.id, done)
        playout.push(floats([0.5] * 4_800))
        assert playout.wait_idle(timeout=10)
        assert playout.queued_frames == 0
        done.set()


def test_closing_from_another_thread_during_a_write_does_not_hang(
    backend: _testing.FakeBackend,
) -> None:
    mic = pattie.Microphone("Agent Mic", _backend=backend)
    # Nobody pulls, so this write waits until its timeout.
    writer = threading.Thread(target=mic.write, args=(floats([0.1] * 48_000), 0.3))
    writer.start()
    time.sleep(0.05)

    mic.close()  # waits for the write, which needs the GIL to finish

    writer.join(timeout=5)
    assert not writer.is_alive()
    assert mic.closed


def test_stats_can_be_read_while_a_write_waits(backend: _testing.FakeBackend) -> None:
    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        # Nobody pulls, so this write waits until its timeout (2 s).
        writer = threading.Thread(target=mic.write, args=(floats([0.1] * 48_000), 2.0))
        writer.start()
        time.sleep(0.05)

        started = time.monotonic()
        stats, played = mic.stats, mic.played_ms  # a meter, from another thread
        waited = time.monotonic() - started

        writer.join(timeout=5)
    assert waited < 0.5, f"stats waited {waited:.2f} s for the write"
    assert stats.buffered_frames == backend.lead_frames
    assert played == 0


def test_closing_a_microphone_removes_it_even_with_a_playout_open(
    backend: _testing.FakeBackend,
) -> None:
    mic = pattie.Microphone("Agent Mic", _backend=backend)
    playout = mic.playout()
    playout.push(floats([0.5] * 480))

    mic.close()

    assert backend.system_read(mic.id, 480) is None, "the Device is still live"
    with pytest.raises(ValueError, match="closed"):
        playout.push(floats([0.5] * 480))


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


def test_a_speaker_in_another_format_still_converts(backend: _testing.FakeBackend) -> None:
    with pattie.Speaker(
        "Agent Speaker", format=pattie.AudioFormat(48_000, "pcm16"), _backend=backend
    ) as speaker:
        backend.system_write(speaker.id, floats([0.5] * 480))
        data = speaker.read()

    assert array.array("h", data).tolist() == [16_384] * 480
