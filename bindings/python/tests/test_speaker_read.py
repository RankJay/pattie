"""Audio through the public API: what the system plays reaches a Speaker."""

import array
import threading

import pytest
from system_side import as_floats, floats

import pattie
from pattie import _testing


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


def test_a_speaker_in_another_format_still_converts(backend: _testing.FakeBackend) -> None:
    with pattie.Speaker(
        "Agent Speaker", format=pattie.AudioFormat(48_000, "pcm16"), _backend=backend
    ) as speaker:
        backend.system_write(speaker.id, floats([0.5] * 480))
        data = speaker.read()

    assert array.array("h", data).tolist() == [16_384] * 480
