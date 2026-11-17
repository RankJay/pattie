"""Shared test setup: every test gets its own test backend, a stand-in for the
driver, so the suite runs on any machine."""

import pytest

from pattie import _testing


@pytest.fixture
def backend() -> _testing.FakeBackend:
    """A fresh backend: as far as names and IDs go, a new app."""
    return _testing.FakeBackend()
