"""Formats, the Converter and stable IDs."""

import array

import pytest

import pattie


def test_a_format_says_what_it_is() -> None:
    fmt = pattie.AudioFormat(24_000, "pcm16", channels=2)

    assert (fmt.rate, fmt.encoding, fmt.channels) == (24_000, "pcm16", 2)
    assert fmt.frame_bytes == 4
    assert fmt == pattie.AudioFormat(24_000, "pcm16", 2)
    assert repr(fmt) == "AudioFormat(24000, 'pcm16', channels=2)"
    assert pattie.AudioFormat(48_000) == pattie.AudioFormat.DEVICE


def test_a_rate_pattie_cannot_take_fails_where_it_is_written() -> None:
    with pytest.raises(pattie.UnsupportedFormat, match="kHz"):
        pattie.AudioFormat(7_000)


def test_an_unknown_encoding_is_a_value_error() -> None:
    with pytest.raises(ValueError, match="pcm16"):
        pattie.AudioFormat(48_000, "mp3")  # ty: ignore[invalid-argument-type]


def test_the_converter_turns_pcm16_into_floats() -> None:
    to_float = pattie.Converter(pattie.AudioFormat(48_000, "pcm16"), pattie.AudioFormat.DEVICE)

    out = to_float.convert(array.array("h", [16_384, -16_384]).tobytes())

    assert array.array("f", out).tolist() == pytest.approx([0.5, -0.5])


def test_converting_in_pieces_gives_the_same_as_at_once() -> None:
    source = pattie.AudioFormat(24_000, "pcm16")
    data = array.array("h", [(i * 37) % 2000 - 1000 for i in range(2_400)]).tobytes()

    whole = pattie.Converter(source, pattie.AudioFormat.DEVICE).convert(data)
    pieces = pattie.Converter(source, pattie.AudioFormat.DEVICE)
    # 333 bytes: pieces that end in the middle of a sample.
    joined = b"".join(pieces.convert(data[i : i + 333]) for i in range(0, len(data), 333))

    assert joined == whole


def test_a_stable_id_depends_only_on_kind_name_and_format() -> None:
    first = pattie.stable_id("microphone", "Agent Mic")

    assert first == pattie.stable_id("microphone", "Agent Mic", rate=48_000, channels=1)
    assert first == "M41B28DFC5B21A0BE"
    assert first != pattie.stable_id("speaker", "Agent Mic")
    assert first != pattie.stable_id("microphone", "Agent Mic", rate=96_000)
    assert first != pattie.stable_id("microphone", "Agent Mic", channels=2)
    with pytest.raises(ValueError, match="microphone"):
        pattie.stable_id("camera", "Agent Mic")  # ty: ignore[invalid-argument-type]
