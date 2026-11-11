"""Creating and closing Devices, and how SDK errors reach Python."""

import sys

import pytest

import viaduct
from viaduct import _testing


def test_a_microphone_gets_an_id_and_with_closes_it(backend: _testing.TestBackend) -> None:
    with viaduct.Microphone("Agent Mic", _backend=backend) as mic:
        first_id = mic.id
        assert not mic.closed

    assert mic.closed
    # The name is free again once the first one is closed.
    with viaduct.Microphone("Agent Mic", _backend=backend) as again:
        assert again.id != first_id


def test_a_duplicate_name_raises_duplicate_name(backend: _testing.TestBackend) -> None:
    with (
        viaduct.Microphone("Agent Mic", _backend=backend),
        pytest.raises(viaduct.DuplicateName),
    ):
        viaduct.Microphone("Agent Mic", _backend=backend)


def test_every_sdk_error_is_a_viaduct_error(backend: _testing.TestBackend) -> None:
    with pytest.raises(viaduct.Error) as raised:
        viaduct.Microphone("", _backend=backend)

    assert isinstance(raised.value, viaduct.BadName)


def test_a_speaker_and_a_microphone_may_share_a_name(backend: _testing.TestBackend) -> None:
    with (
        viaduct.Microphone("Agent", _backend=backend) as mic,
        viaduct.Speaker("Agent", _backend=backend) as speaker,
    ):
        assert mic.id != speaker.id


def test_using_a_closed_device_raises_value_error(backend: _testing.TestBackend) -> None:
    mic = viaduct.Microphone("Agent Mic", _backend=backend)
    mic.close()
    mic.close()  # closing twice is fine

    with pytest.raises(ValueError, match="closed"):
        mic.write(b"\x00" * 4)


@pytest.mark.skipif(sys.platform != "win32", reason="needs Windows without the driver")
def test_without_the_driver_the_error_says_how_to_install_it() -> None:
    with pytest.raises(viaduct.DriverNotInstalled, match="viaduct-cli install"):
        viaduct.Microphone("Agent Mic")
