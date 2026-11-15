"""How SDK errors reach Python: each one as its own exception under
pattie.Error, with the fix in its message where there is one."""

import pytest

import pattie
from pattie import _testing


def test_a_removed_driver_is_its_own_error_under_error() -> None:
    # ADR 0024: the driver was removed or is being updated while a Device was
    # live. Its own exception, so an app can recreate its Devices; still a
    # pattie.Error, so existing handlers keep catching it.
    assert issubclass(pattie.DriverRemoved, pattie.Error)
    error = _testing.sdk_error("DriverRemoved")
    assert type(error) is pattie.DriverRemoved
    assert "create the Device again" in str(error)
    with pytest.raises(pattie.Error):
        raise error


def test_a_driver_not_installed_still_says_how_to_install_it() -> None:
    error = _testing.sdk_error("DriverNotInstalled")
    assert type(error) is pattie.DriverNotInstalled
    assert "pattie-cli install" in str(error)
