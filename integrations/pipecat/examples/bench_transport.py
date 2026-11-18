"""What the transport costs, per 10 s of audio, on Pattie's test backend: no
driver, the system's side played on its own threads in real time.

- input: the meeting plays 10 ms packets to the Speaker; the pipeline should get
  a 20 ms frame every 20 ms. Reports frames received, the longest gap between
  two, the process's CPU, and the longest any other Python thread waited for
  the GIL meanwhile.
- input with a busy pool: the same, while the app's own default thread pool is
  kept full (blocking jobs, as an app's `asyncio.to_thread` calls are).
- output: the pipeline speaks 20 ms frames of 24 kHz PCM16; the system takes
  them at 48 kHz. Reports CPU and the GIL figure.

Run from integrations/pipecat: `uv run python examples/bench_transport.py LABEL`.
"""

import array
import asyncio
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor

from pattie import _testing
from pipecat.frames.frames import EndFrame, Frame, InputAudioRawFrame, OutputAudioRawFrame
from pipecat.pipeline.pipeline import Pipeline
from pipecat.pipeline.runner import WorkerRunner
from pipecat.pipeline.worker import PipelineParams, PipelineWorker
from pipecat.processors.frame_processor import FrameDirection, FrameProcessor
from pipecat.tests.utils import run_test

from pipecat_pattie import PattieTransport, PattieTransportParams

SECONDS = 10


class GilProbe:
    """A thread sleeping 1 ms at a time: how late it wakes is how long the GIL
    was held elsewhere."""

    def __init__(self) -> None:
        self.late: list[float] = []
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()

    def _run(self) -> None:
        while not self._stop.is_set():
            t = time.perf_counter()
            time.sleep(0.001)
            self.late.append((time.perf_counter() - t - 0.001) * 1e3)

    def stop(self) -> tuple[float, float]:
        self._stop.set()
        self._thread.join()
        self.late.sort()
        return self.late[-1], self.late[int(0.99 * (len(self.late) - 1))]


class Arrivals(FrameProcessor):
    """Notes when each input audio frame reaches the pipeline."""

    def __init__(self) -> None:
        super().__init__()
        self.at: list[float] = []

    async def process_frame(self, frame: Frame, direction: FrameDirection) -> None:
        await super().process_frame(frame, direction)
        if isinstance(frame, InputAudioRawFrame):
            self.at.append(time.perf_counter())
        await self.push_frame(frame, direction)


def play_to(backend: _testing.FakeBackend, speaker_id: int, stop: threading.Event) -> None:
    """The meeting app: 10 ms packets on the wall clock."""
    packet = array.array("f", [0.3] * 480).tobytes()
    started, sent = time.perf_counter(), 0
    while not stop.is_set():
        due = int((time.perf_counter() - started) * 100)
        while sent < due:
            backend.system_write(speaker_id, packet)
            sent += 1
        time.sleep(0.002)


async def bench_input(busy_pool: bool) -> str:
    backend = _testing.FakeBackend()
    transport = PattieTransport(
        PattieTransportParams(audio_in_enabled=True, audio_in_sample_rate=16_000),
        _backend=backend,
    )
    assert transport.speaker is not None
    stop = threading.Event()
    player = threading.Thread(target=play_to, args=(backend, transport.speaker.id, stop))
    loop = asyncio.get_running_loop()
    pool = ThreadPoolExecutor(max_workers=8)
    loop.set_default_executor(pool)
    blockers: list[asyncio.Future[None]] = []
    if busy_pool:
        # The app's own work: more blocking jobs than the pool has threads.
        blockers = [loop.run_in_executor(None, time.sleep, SECONDS + 2) for _ in range(32)]
    arrivals = Arrivals()
    probe = GilProbe()
    cpu0 = time.process_time()
    player.start()
    worker = PipelineWorker(
        Pipeline([transport.input(), arrivals]),
        params=PipelineParams(audio_in_sample_rate=16_000),
        idle_timeout_secs=None,
    )
    runner = WorkerRunner(handle_sigint=False)

    async def end_later() -> None:
        await asyncio.sleep(SECONDS)
        await worker.queue_frame(EndFrame())

    await asyncio.gather(runner.run(worker), end_later())
    cpu = time.process_time() - cpu0
    stop.set()
    player.join()
    gil_max, gil_p99 = probe.stop()
    for b in blockers:
        b.cancel()
    pool.shutdown(wait=False, cancel_futures=True)
    gaps = [b - a for a, b in zip(arrivals.at, arrivals.at[1:], strict=False)]
    worst_gap = max(gaps) * 1e3 if gaps else float("nan")
    return (
        f"frames {len(arrivals.at):>4} (expected ~{SECONDS * 50}), "
        f"longest gap {worst_gap:6.1f} ms, CPU {cpu:5.2f} s ({100 * cpu / SECONDS:4.1f}%), "
        f"GIL held max {gil_max:5.1f} ms, p99 {gil_p99:4.2f}"
    )


async def bench_output() -> str:
    backend = _testing.FakeBackend()
    transport = PattieTransport(
        PattieTransportParams(audio_out_enabled=True, audio_out_sample_rate=24_000),
        _backend=backend,
    )
    assert transport.microphone is not None
    # The system recording the microphone at 48 kHz in real time.
    stop = threading.Event()
    mic_id = transport.microphone.id

    def pull() -> None:
        started, pulled = time.perf_counter(), 0
        while not stop.is_set():
            due = int((time.perf_counter() - started) * 100)
            while pulled < due:
                backend.system_read(mic_id, 480)
                pulled += 1
            time.sleep(0.002)

    puller = threading.Thread(target=pull)
    chunk = array.array("h", [6_000] * 480).tobytes()  # 20 ms at 24 kHz
    frames = [
        OutputAudioRawFrame(audio=chunk, sample_rate=24_000, num_channels=1)
        for _ in range(SECONDS * 50)
    ]
    probe = GilProbe()
    cpu0, wall0 = time.process_time(), time.perf_counter()
    puller.start()
    await run_test(transport.output(), frames_to_send=frames)
    cpu, wall = time.process_time() - cpu0, time.perf_counter() - wall0
    stop.set()
    puller.join()
    gil_max, gil_p99 = probe.stop()
    return (
        f"wall {wall:5.1f} s for {SECONDS} s of speech, "
        f"CPU {cpu:5.2f} s ({100 * cpu / wall:4.1f}%), "
        f"GIL held max {gil_max:5.1f} ms, p99 {gil_p99:4.2f}"
    )


async def main() -> None:
    label = sys.argv[1] if len(sys.argv) > 1 else "run"
    print(f"{label:<7} input           : {await bench_input(busy_pool=False)}", flush=True)
    print(f"{label:<7} input, busy pool: {await bench_input(busy_pool=True)}", flush=True)
    print(f"{label:<7} output          : {await bench_output()}", flush=True)


if __name__ == "__main__":
    import logging

    from loguru import logger

    logger.remove()
    logging.getLogger("pattie").setLevel(logging.ERROR)
    asyncio.run(main())
