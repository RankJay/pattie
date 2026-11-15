"""Shared setup: Pattie's test backend plays the system's side of each Device
(Windows' audio engine, or PipeWire), so the transport runs without a driver."""

import array
import threading
import time

import pytest
from pattie import _testing


@pytest.fixture
def backend() -> _testing.TestBackend:
    """A fresh backend: as far as names and IDs go, a new app."""
    return _testing.TestBackend()


def pcm16(value: float, frames: int) -> bytes:
    """`frames` mono 16-bit samples of a constant `value` (-1.0 to 1.0)."""
    return array.array("h", [round(value * 32_767)] * frames).tobytes()


def f32(values: list[float]) -> bytes:
    return array.array("f", values).tobytes()


class SystemPull:
    """The system recording a microphone at 48 kHz in real time, as a sound
    card does: on its own thread, it takes whatever 10 ms packets the wall
    clock says are due."""

    def __init__(self, backend: _testing.TestBackend, device_id: int) -> None:
        self.taken: list[float] = []
        self._stop = threading.Event()

        def run() -> None:
            started, pulled = time.monotonic(), 0
            while not self._stop.is_set():
                due = int((time.monotonic() - started) * 100)  # packets so far
                while pulled < due:
                    got = backend.driver_read(device_id, 480)
                    if got is not None:
                        self.taken.extend(array.array("f", got).tolist())
                    pulled += 1
                time.sleep(0.002)

        self._thread = threading.Thread(target=run, daemon=True)
        self._thread.start()

    def stop(self) -> list[float]:
        self._stop.set()
        self._thread.join()
        return self.taken
