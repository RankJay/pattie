"""close() never hangs behind a call that waits, and a meter's reads (stats,
clock) never wait for one. Each blocking call waits in short pieces and stops
once close() is called."""

import array
import threading
import time
from collections.abc import Callable

import pattie
from pattie import _testing

# close() should end a waiting call within about one 100 ms piece.
PROMPT = 0.5


def floats(values: list[float]) -> bytes:
    return array.array("f", values).tobytes()


def run_in_thread(call: Callable[[], object]) -> tuple[threading.Thread, list[object]]:
    result: list[object] = []
    thread = threading.Thread(target=lambda: result.append(call()), daemon=True)
    thread.start()
    time.sleep(0.1)  # let it start waiting
    return thread, result


def close_promptly(close: Callable[[], None], thread: threading.Thread) -> float:
    """Closes from another thread with a time limit, so a close() that hangs
    fails the test instead of hanging the suite."""
    started = time.monotonic()
    closer = threading.Thread(target=close, daemon=True)
    closer.start()
    closer.join(timeout=3)
    assert not closer.is_alive(), "close() hung behind the waiting call"
    thread.join(timeout=3)
    assert not thread.is_alive(), "the waiting call never returned"
    return time.monotonic() - started


def test_close_ends_a_write_that_waits_forever(backend: _testing.FakeBackend) -> None:
    mic = pattie.Microphone("Agent Mic", _backend=backend)
    # Nobody records, so without close() this would never finish.
    thread, result = run_in_thread(lambda: mic.write(floats([0.1] * 48_000)))

    took = close_promptly(mic.close, thread)

    assert took < PROMPT, f"close() took {took:.2f} s"
    assert result == [backend.lead_frames], "it returns how much went in"


def test_close_ends_a_wait_for_playout_that_waits_forever(
    backend: _testing.FakeBackend,
) -> None:
    mic = pattie.Microphone("Agent Mic", _backend=backend)
    mic.write_nowait(floats([0.1] * 480))
    thread, result = run_in_thread(mic.wait_for_playout)

    took = close_promptly(mic.close, thread)

    assert took < PROMPT, f"close() took {took:.2f} s"
    assert result == [False]


def test_close_ends_a_read_that_waits_forever(backend: _testing.FakeBackend) -> None:
    speaker = pattie.Speaker("Agent Speaker", _backend=backend)
    thread, result = run_in_thread(lambda: speaker.read(timeout=None))

    took = close_promptly(speaker.close, thread)

    assert took < PROMPT, f"close() took {took:.2f} s"
    assert result == [b""]
    assert backend.system_write(speaker.id, floats([0.1] * 10)) is False, "still live"


def test_close_ends_a_read_timed_that_waits_forever(backend: _testing.FakeBackend) -> None:
    speaker = pattie.Speaker("Agent Speaker", _backend=backend)
    thread, result = run_in_thread(lambda: speaker.read_timed(timeout=None))

    took = close_promptly(speaker.close, thread)

    assert took < PROMPT, f"close() took {took:.2f} s"
    assert result == [(b"", None)]


def test_a_speakers_stats_and_clock_never_wait_for_a_read(
    backend: _testing.FakeBackend,
) -> None:
    with pattie.Speaker("Agent Speaker", _backend=backend) as speaker:
        backend.system_write(speaker.id, floats([0.1] * 480))
        speaker.read()
        thread, _ = run_in_thread(lambda: speaker.read(timeout=2))

        started = time.monotonic()
        stats, clock = speaker.stats, speaker.clock  # a meter, from another thread
        waited = time.monotonic() - started

        backend.system_write(speaker.id, floats([0.1] * 480))
        thread.join(timeout=5)
    assert waited < 0.1, f"waited {waited:.2f} s for the read"
    assert stats.buffered_frames == 0
    assert clock is not None
    assert clock.frames == 480


def test_a_write_still_finishes_when_listeners_take_it(backend: _testing.FakeBackend) -> None:
    """Waiting in pieces mustn't cut a write short: with a listener, it all goes in."""
    with pattie.Microphone("Agent Mic", _backend=backend) as mic:
        done = threading.Event()

        def listen() -> None:
            while not done.is_set():
                backend.system_read(mic.id, 480)
                time.sleep(0.005)

        threading.Thread(target=listen, daemon=True).start()
        written = mic.write(floats([0.2] * 24_000))  # 500 ms: several pieces
        done.set()
    assert written == 24_000


def close_mid_piece(call: Callable[[], object], close: Callable[[], None]) -> float:
    """Starts `call` in a thread, closes 50 ms in (halfway through its first
    100 ms piece), and returns how long close() took to end it."""
    thread = threading.Thread(target=call, daemon=True)
    thread.start()
    time.sleep(0.05)
    return close_promptly(close, thread)


def test_close_ends_a_write_at_once_not_at_its_piece(backend: _testing.FakeBackend) -> None:
    mic = pattie.Microphone("Agent Mic", _backend=backend)

    took = close_mid_piece(lambda: mic.write(floats([0.1] * 48_000)), mic.close)

    assert took < 0.025, f"close() took {took * 1000:.0f} ms: the write ran out its piece"


def test_close_ends_a_read_at_once_not_at_its_piece(backend: _testing.FakeBackend) -> None:
    speaker = pattie.Speaker("Agent Speaker", _backend=backend)

    took = close_mid_piece(lambda: speaker.read(timeout=None), speaker.close)

    assert took < 0.025, f"close() took {took * 1000:.0f} ms: the read ran out its piece"
