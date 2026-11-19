"""A Playout through the public API, and closing a Microphone while another
thread writes or a Playout is open."""

import threading
import time

import pytest
from system_side import as_floats, floats, pull_in_background

import pattie
from pattie import _testing


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


def test_closing_the_microphone_ends_a_playouts_wait_idle_and_frees_the_name(
    backend: _testing.FakeBackend,
) -> None:
    mic = pattie.Microphone("Agent Mic", _backend=backend)
    playout = mic.playout()
    playout.push(floats([0.1] * 48_000))  # nobody pulls: never idle
    result: list[bool] = []
    waiter = threading.Thread(target=lambda: result.append(playout.wait_idle()), daemon=True)
    waiter.start()
    time.sleep(0.05)

    mic.close()

    # The wait lets go within about one 100 ms piece, and with it the Device:
    # until 2026-11-19 it held both until its timeout (a year when None).
    waiter.join(timeout=1)
    assert not waiter.is_alive()
    assert result == [False]
    pattie.Microphone("Agent Mic", _backend=backend).close()


def test_interrupt_names_the_reply_being_heard(backend: _testing.FakeBackend) -> None:
    with pattie.Microphone("Agent Mic", _backend=backend) as mic, mic.playout() as playout:
        assert playout.push(floats([0.5] * 4_800), reply="item_1") == 4_800
        assert playout.push(floats([0.25] * 4_800), reply="item_2") == 4_800
        deadline = time.monotonic() + 5
        taken = 0
        while taken < 960 and time.monotonic() < deadline:
            got = backend.system_read(mic.id, 480)
            assert got is not None
            taken += sum(1 for x in as_floats(got) if x != 0)
            time.sleep(0.005)
        cut = playout.interrupt()

    assert cut.reply == "item_1"
    assert 960 <= cut.heard_frames < 4_800
    assert "item_1" in repr(cut)


def test_unnamed_audio_reports_no_reply(backend: _testing.FakeBackend) -> None:
    with pattie.Microphone("Agent Mic", _backend=backend) as mic, mic.playout() as playout:
        playout.push(floats([0.5] * 4_800))
        assert playout.interrupt().reply is None
