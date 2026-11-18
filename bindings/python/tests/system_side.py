"""Sample bytes as the system's side reads and writes them, and the system's
pulls played by a Python thread. Shared by the audio tests."""

import array
import threading
import time

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
