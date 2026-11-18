"""A/B measurement of AsyncMicrophone.write on long buffers (measurements.md, run h).

The loop before the quadratic-copy fix (each 100 ms wait handed the whole
remainder of the buffer) against the loop after it (each wait handed at most
200 ms). Both run in one process on one build, so only the loop differs.

Guest (real driver): just run it. It starts listen.ps1 (next to this script),
which records from "Agent Mic" as a meeting app would; --manual records with
Sound Recorder instead. Host (no driver): --fake, run with the binding's dev
build (uv run, in bindings/python), where a thread takes 10 ms every 10 ms on
the wall clock, catching up after a delay as a sound card would.

Per run:
- wall time (should equal the audio's length) and process CPU, all threads;
- calls to the native write() and the bytes handed to them: the binding copies
  what it's handed twice with the GIL held (to bytes, then to samples), so the
  bytes copied are twice the bytes handed;
- the worst and p99 time a thread sleeping 1 ms at a time woke late: how long
  the GIL, and so the event loop, was held elsewhere;
- the Gap and Overflow the microphone counted during the write;
- with the listener, what the recording side saw.
"""

import argparse
import array
import asyncio
import logging
import math
import subprocess
import sys
import threading
import time
from collections.abc import Buffer
from pathlib import Path
from typing import Any

from pattie.aio import AsyncMicrophone

RATE = 48_000
PIECE_SECONDS = 0.1  # pattie.aio.pieces._PIECE_SECONDS
MB = 1e6


class CountingMicrophone:
    """The native Microphone, counting the calls to write() and the bytes handed."""

    def __init__(self, mic: Any) -> None:
        self.mic = mic
        self.calls = 0
        self.handed = 0

    def write(self, data: Buffer, timeout: float) -> int:
        self.calls += 1
        self.handed += memoryview(data).nbytes
        return self.mic.write(data, timeout)

    def __getattr__(self, name: str) -> Any:
        return getattr(self.mic, name)


async def write_old(mic: Any, data: bytes) -> int:
    """The loop before the fix, as it was apart from names."""
    samples = memoryview(mic._input.convert(data))
    frame_bytes = mic._frame_bytes
    written = 0
    while written * frame_bytes < len(samples):
        rest = samples[written * frame_bytes :]
        written += await asyncio.to_thread(mic._mic.write, rest, PIECE_SECONDS)
    return written


async def write_new(mic: Any, data: bytes) -> int:
    return await mic.write(data)


def gil_probe(stop: threading.Event, lateness_ms: list[float]) -> None:
    """Sleeps 1 ms at a time (a high-resolution timer since Python 3.11) and
    records how late it got back. (asyncio.sleep can't show it on Windows: its
    timer ticks every 15.6 ms.)"""
    period = 0.001
    while not stop.is_set():
        start = time.perf_counter()
        time.sleep(period)
        lateness_ms.append((time.perf_counter() - start - period) * 1e3)


def tone(seconds: float) -> bytes:
    step = 2 * math.pi * 440 / RATE
    samples = (0.2 * math.sin(step * i) for i in range(int(RATE * seconds)))
    return array.array("f", samples).tobytes()


def clock_puller(backend: Any, device_id: int, stop: threading.Event) -> None:
    """The Fake backend's system side, paced by the wall clock: what's due is
    taken, late or not, so a writer held up by the GIL shows as Gap."""
    started = time.perf_counter()
    taken = 0
    while not stop.is_set():
        due = int((time.perf_counter() - started) * RATE)
        while taken + 480 <= due:
            backend.system_read(device_id, 480)
            taken += 480
        time.sleep(0.010)


async def one_run(mic: Any, impl: str, seconds: float) -> dict[str, float]:
    data = tone(seconds)
    write = write_old if impl == "old" else write_new
    counting = CountingMicrophone(mic._mic)
    mic._mic = counting
    lateness: list[float] = []
    stop = threading.Event()
    probe = threading.Thread(target=gil_probe, args=(stop, lateness))
    probe.start()
    before = mic.stats
    cpu0, wall0 = time.process_time(), time.perf_counter()
    try:
        frames = await write(mic, data)
    finally:
        wall, cpu = time.perf_counter() - wall0, time.process_time() - cpu0
        mic._mic = counting.mic
        stop.set()
        probe.join()
    after = mic.stats
    await mic.wait_for_playout()
    lateness.sort()
    return {
        "frames": frames,
        "wall_s": wall,
        "cpu_s": cpu,
        "calls": counting.calls,
        "handed_mb": counting.handed / MB,
        "gil_max_ms": lateness[-1],
        "gil_p99_ms": lateness[int(0.99 * (len(lateness) - 1))],
        "gil_over_20ms": sum(1 for x in lateness if x > 20),
        "gap": after.gap_frames - before.gap_frames,
        "overflow": after.overflow_frames - before.overflow_frames,
    }


class Listener:
    """listen.ps1 recording from the endpoint as a meeting app would, so the
    Device's stream runs."""

    def __init__(self, name: str) -> None:
        script = Path(__file__).with_name("listen.ps1")
        command = ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]
        self._process = subprocess.Popen(
            [*command, str(script), "-Name", name],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        print(f'waiting for the "{name}" endpoint (up to 60 s) ...', flush=True)
        line = self._line()
        if not line.startswith("listening:"):
            raise SystemExit(f"listen.ps1: {line or 'exited'}")
        print(line, flush=True)
        time.sleep(2)  # let the stream settle before the first run

    def _line(self) -> str:
        assert self._process.stdout is not None
        return self._process.stdout.readline().strip()

    def _send(self, command: str) -> str:
        assert self._process.stdin is not None
        self._process.stdin.write(command + "\n")
        self._process.stdin.flush()
        return self._line()

    def mark(self) -> str:
        """Counts since the previous mark."""
        return self._send("mark").removeprefix("counts: ")

    def close(self) -> str:
        line = self._send("stop")
        self._process.wait(timeout=10)
        return f"listener {line}"


async def main(args: argparse.Namespace) -> None:
    backend = None
    if args.fake:
        from pattie import _testing  # noqa: PLC0415 (only the dev build has it)

        backend = _testing.FakeBackend()
    async with AsyncMicrophone("Agent Mic", _backend=backend) as mic:
        stop_pull = threading.Event()
        if backend is not None:
            puller = threading.Thread(
                target=clock_puller, args=(backend, mic.id, stop_pull), daemon=True
            )
            puller.start()
        listener = None
        if args.manual:
            prompt = '"Agent Mic" is up. Record from it in Sound Recorder, then press Enter.'
            await asyncio.to_thread(input, prompt)
        elif backend is None or args.listen:
            listener = Listener(args.listen or "Agent Mic")
        print(
            f"{'impl':<5}{'audio s':>8}{'wall s':>8}{'CPU s':>8}{'CPU %':>7}{'calls':>7}"
            f"{'MB handed':>11}{'GIL max ms':>11}{'p99 ms':>8}{'>20ms':>6}{'Gap':>7}"
            f"{'Ovfl':>6}" + ("  | listener" if listener else "")
        )
        for seconds in args.seconds:
            for impl in args.impl:
                for _ in range(args.repeat):
                    if listener:
                        listener.mark()  # what follows belongs to this run
                    r = await one_run(mic, impl, seconds)
                    tail = "" if r["frames"] == int(RATE * seconds) else "  SHORT WRITE"
                    if listener:
                        tail += "  | " + listener.mark()
                    print(
                        f"{impl:<5}{seconds:>8.0f}{r['wall_s']:>8.2f}{r['cpu_s']:>8.2f}"
                        f"{100 * r['cpu_s'] / r['wall_s']:>7.1f}{r['calls']:>7.0f}"
                        f"{r['handed_mb']:>11.1f}{r['gil_max_ms']:>11.1f}"
                        f"{r['gil_p99_ms']:>8.2f}{r['gil_over_20ms']:>6.0f}{r['gap']:>7.0f}"
                        f"{r['overflow']:>6.0f}{tail}",
                        flush=True,
                    )
                    await asyncio.sleep(1)
        stop_pull.set()
        if listener:
            print(listener.close())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--seconds", type=float, nargs="+", default=[60, 180])
    parser.add_argument("--impl", nargs="+", choices=["old", "new"], default=["old", "new"])
    parser.add_argument("--repeat", type=int, default=1)
    parser.add_argument("--fake", action="store_true", help="no driver: the Fake backend")
    parser.add_argument("--listen", help="the capture endpoint to record (default Agent Mic)")
    parser.add_argument("--manual", action="store_true", help="record with Sound Recorder")
    print(f"Python {sys.version.split()[0]}")
    # Loss is counted in the table; a warning per lost piece would bury it.
    logging.getLogger("pattie").setLevel(logging.ERROR)
    asyncio.run(main(parser.parse_args()))
