"""Does a big Playout.push starve the playout's feeder? (measurements.md, run i)

A voice agent's long reply pushed in 10 s pieces into a Microphone's playout,
at 8 channels and 96 kHz by default: before the fix, each push grew one queue
under the lock the feeder needs every 10 ms, and a regrowth took longer than
the 20 ms lead. A queue keeps its size after an interrupt, so with one playout
only the first reply (or one longer than any before) grows it: --fresh makes
every reply a new playout's first. Run it once per build (install the "before" wheel, run with
--label before; then the "after" wheel, --label after).

Guest (real driver): just run it; it starts listen.ps1 (next to this script),
which records from "Agent Mic" as a meeting app does. Host (no driver): --fake,
with the binding's dev build, where a thread takes 10 ms every 10 ms on the wall
clock.

Per reply: the Gap and Overflow the microphone counted, the longest push(), the
longest another Python thread waited for the GIL meanwhile (push() converts with
it released since the binding fix), and with the listener, what the recording
side saw.
"""

import argparse
import array
import logging
import sys
import threading
import time
from typing import Any

import pattie
from aio_write_measure import Listener, gil_probe

PIECES = 6  # 60 s per reply
PIECE_SECONDS = 10
PLAY_SECONDS = 8  # each reply plays this long, then is interrupted


def clock_puller(backend: Any, device_id: int, rate: int, stop: threading.Event) -> None:
    started = time.perf_counter()
    packet = rate // 100
    taken = 0
    while not stop.is_set():
        due = int((time.perf_counter() - started) * rate)
        while taken + packet <= due:
            backend.system_read(device_id, packet)
            taken += packet
        time.sleep(0.002)


def one_reply(mic: Any, playout: Any, piece: bytes) -> dict[str, float]:
    before = mic.stats
    lateness: list[float] = []
    stop = threading.Event()
    probe = threading.Thread(target=gil_probe, args=(stop, lateness))
    probe.start()
    started = time.perf_counter()
    longest = 0.0
    for _ in range(PIECES):
        t = time.perf_counter()
        playout.push(piece)
        longest = max(longest, time.perf_counter() - t)
        time.sleep(0.1)
    stop.set()
    probe.join()
    time.sleep(max(0.0, PLAY_SECONDS - (time.perf_counter() - started)))
    after = mic.stats
    playout.interrupt()
    return {
        "gap": after.gap_frames - before.gap_frames,
        "overflow": after.overflow_frames - before.overflow_frames,
        "push_ms": longest * 1e3,
        "gil_ms": max(lateness),
    }


def main(args: argparse.Namespace) -> None:
    backend = None
    if args.fake:
        from pattie import _testing  # noqa: PLC0415 (only the dev build has it)

        backend = _testing.FakeBackend()
    mic = pattie.Microphone("Agent Mic", rate=args.rate, channels=args.channels, _backend=backend)
    stop = threading.Event()
    if backend is not None:
        puller = threading.Thread(
            target=clock_puller, args=(backend, mic.id, args.rate, stop), daemon=True
        )
        puller.start()
    listener = Listener("Agent Mic") if backend is None else None
    playout = None if args.fresh else mic.playout()
    piece = array.array("f", [0.2] * (args.rate * PIECE_SECONDS * args.channels)).tobytes()
    packet = args.rate // 100
    print(
        f"{args.label}: {args.channels} ch, {args.rate} Hz, replies of {PIECES} x "
        f"{PIECE_SECONDS} s pushed 100 ms apart, {PLAY_SECONDS} s played each, "
        + ("a new playout for each" if args.fresh else "one playout for all")
    )
    total_gap, hit, worst_push, worst_gil = 0, 0, 0.0, 0.0
    for n in range(1, args.replies + 1):
        if args.fresh:
            playout = mic.playout()
        time.sleep(0.5)  # steady state: the feeder already sending silence
        if listener:
            listener.mark()
        r = one_reply(mic, playout, piece)
        if args.fresh:
            playout.close()
        tail = f"  | {listener.mark()}" if listener else ""
        total_gap += int(r["gap"])
        hit += r["gap"] > 0
        worst_push = max(worst_push, r["push_ms"])
        worst_gil = max(worst_gil, r["gil_ms"])
        print(
            f"{args.label:<7}reply {n:>2}: Gap {int(r['gap']):>6} frames, "
            f"Overflow {int(r['overflow']):>6}, longest push {r['push_ms']:>6.1f} ms, "
            f"GIL held {r['gil_ms']:>5.1f} ms{tail}",
            flush=True,
        )
        time.sleep(0.5)
    print(
        f"{args.label}: Gap {total_gap / packet:.1f} packets in all, {hit}/{args.replies} "
        f"replies hit; longest push {worst_push:.1f} ms; GIL held at most {worst_gil:.1f} ms"
    )
    if playout and not args.fresh:
        playout.close()
    stop.set()
    if listener:
        print(listener.close())
    mic.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--label", default="run")
    parser.add_argument("--rate", type=int, default=96_000)
    parser.add_argument("--channels", type=int, default=8)
    parser.add_argument("--replies", type=int, default=10)
    parser.add_argument("--fake", action="store_true", help="no driver: the Fake backend")
    parser.add_argument(
        "--fresh",
        action="store_true",
        help="a new playout per reply: each reply is a playout's first, which grows its queue",
    )
    print(f"Python {sys.version.split()[0]}")
    logging.getLogger("pattie").setLevel(logging.ERROR)
    main(parser.parse_args())
