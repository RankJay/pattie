"""Formats, the Converter and stable IDs."""

import array

import pytest

import viaduct


def test_a_format_says_what_it_is() -> None:
    fmt = viaduct.Format(24_000, "pcm16", channels=2)

    assert (fmt.rate, fmt.encoding, fmt.channels) == (24_000, "pcm16", 2)
    assert fmt.frame_bytes == 4
    assert fmt == viaduct.Format(24_000, "pcm16", 2)
    assert repr(fmt) == "Format(24000, 'pcm16', channels=2)"
    assert viaduct.Format(48_000) == viaduct.Format.DEVICE


def test_a_rate_viaduct_cannot_take_fails_where_it_is_written() -> None:
    with pytest.raises(viaduct.UnsupportedFormat, match="kHz"):
        viaduct.Format(7_000)


def test_an_unknown_encoding_is_a_value_error() -> None:
    with pytest.raises(ValueError, match="pcm16"):
        viaduct.Format(48_000, "mp3")  # ty: ignore[invalid-argument-type]


def test_the_converter_turns_pcm16_into_floats() -> None:
    to_float = viaduct.Converter(viaduct.Format(48_000, "pcm16"), viaduct.Format.DEVICE)

    out = to_float.convert(array.array("h", [16_384, -16_384]).tobytes())

    assert array.array("f", out).tolist() == pytest.approx([0.5, -0.5])


def test_converting_in_pieces_gives_the_same_as_at_once() -> None:
    source = viaduct.Format(24_000, "pcm16")
    data = array.array("h", [(i * 37) % 2000 - 1000 for i in range(2_400)]).tobytes()

    whole = viaduct.Converter(source, viaduct.Format.DEVICE).convert(data)
    pieces = viaduct.Converter(source, viaduct.Format.DEVICE)
    # 333 bytes: pieces that end in the middle of a sample.
    joined = b"".join(pieces.convert(data[i : i + 333]) for i in range(0, len(data), 333))

    assert joined == whole


def test_a_stable_id_depends_only_on_kind_name_and_format() -> None:
    first = viaduct.stable_id("microphone", "Agent Mic")

    assert first == viaduct.stable_id("microphone", "Agent Mic", rate=48_000, channels=1)
    assert first == "M41B28DFC5B21A0BE"
    assert first != viaduct.stable_id("speaker", "Agent Mic")
    assert first != viaduct.stable_id("microphone", "Agent Mic", rate=96_000)
    assert first != viaduct.stable_id("microphone", "Agent Mic", channels=2)
    with pytest.raises(ValueError, match="microphone"):
        viaduct.stable_id("camera", "Agent Mic")  # ty: ignore[invalid-argument-type]
