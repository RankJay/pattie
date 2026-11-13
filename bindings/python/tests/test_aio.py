"""The asyncio classes: waiting never blocks the event loop."""

import array
import asyncio
import time

import pytest

from viaduct import _testing
from viaduct.aio import AsyncMicrophone, AsyncSpeaker


def floats(values: list[float]) -> bytes:
    return array.array("f", values).tobytes()


async def pull(backend: _testing.TestBackend, device_id: int, until: asyncio.Event) -> int:
    """The system's part, as a task on the same loop: it only runs if the
    microphone's waits leave the loop free."""
    taken = 0
    while not until.is_set():
        got = backend.driver_read(device_id, 480)
        taken += len(got or b"") // 4
        await asyncio.sleep(0.005)
    return taken


def test_an_async_write_leaves_the_loop_free(backend: _testing.TestBackend) -> None:
    async def main() -> None:
        async with AsyncMicrophone("Agent Mic", _backend=backend) as mic:
            done = asyncio.Event()
            puller = asyncio.create_task(pull(backend, mic.id, done))
            assert await mic.write(floats([0.5] * 24_000)) == 24_000
            async with asyncio.timeout(10):
                await mic.wait_for_playout()
            done.set()
            assert await puller >= 24_000

    asyncio.run(main())


def test_cancelling_a_write_stops_it_soon(backend: _testing.TestBackend) -> None:
    async def main() -> None:
        async with AsyncMicrophone("Agent Mic", _backend=backend) as mic:
            # Nobody pulls, so this write could never finish.
            write = asyncio.create_task(mic.write(floats([0.5] * 480_000)))
            await asyncio.sleep(0.05)
            started = time.monotonic()
            write.cancel()
            with pytest.raises(asyncio.CancelledError):
                await write
            assert time.monotonic() - started < 1

    asyncio.run(main())


def test_an_async_speaker_yields_what_was_played(backend: _testing.TestBackend) -> None:
    async def main() -> bytes:
        async with AsyncSpeaker("Agent Speaker", _backend=backend) as speaker:
            loop = asyncio.get_running_loop()
            loop.call_later(0.05, backend.driver_write, speaker.id, floats([0.25] * 480))
            async for chunk in speaker:
                return chunk
        return b""

    heard = asyncio.run(main())

    assert array.array("f", heard).tolist() == pytest.approx([0.25] * 480)


def test_an_async_playout_waits_idle_without_blocking(backend: _testing.TestBackend) -> None:
    async def main() -> None:
        async with AsyncMicrophone("Agent Mic", _backend=backend) as mic:
            done = asyncio.Event()
            puller = asyncio.create_task(pull(backend, mic.id, done))
            async with mic.playout() as playout:
                playout.push(floats([0.5] * 4_800))
                async with asyncio.timeout(10):
                    await playout.wait_idle()
            done.set()
            await puller

    asyncio.run(main())


def test_asyncio_timeout_gives_up_on_a_wait_soon(backend: _testing.TestBackend) -> None:
    async def main() -> None:
        async with AsyncMicrophone("Agent Mic", _backend=backend) as mic:
            mic.write_nowait(floats([0.5] * 480))  # nobody pulls, so it never plays out
            started = time.monotonic()
            with pytest.raises(TimeoutError):
                async with asyncio.timeout(0.05):
                    await mic.wait_for_playout()
            assert time.monotonic() - started < 1

    asyncio.run(main())


def test_an_async_live_writer_relays_audio_without_waiting(
    backend: _testing.TestBackend,
) -> None:
    async def main() -> None:
        async with AsyncMicrophone("Agent Mic", _backend=backend) as mic:
            live = mic.live_writer()
            assert live.push(floats([0.5] * 9_600)) == 9_600  # never waits
            done = asyncio.Event()
            puller = asyncio.create_task(pull(backend, mic.id, done))
            await asyncio.sleep(0.3)
            done.set()
            assert await puller > 0
            assert mic.clock is not None
            assert mic.next_play_time is not None

    asyncio.run(main())


def test_an_async_read_timed_says_when_it_was_played(backend: _testing.TestBackend) -> None:
    async def main() -> tuple[bytes, float | None, float]:
        async with AsyncSpeaker("Agent Speaker", _backend=backend) as speaker:
            loop = asyncio.get_running_loop()
            loop.call_later(0.05, backend.driver_write, speaker.id, floats([0.25] * 480))
            data, first_at = await speaker.read_timed()
            return data, first_at, time.perf_counter()

    data, first_at, after = asyncio.run(main())

    assert len(data) == 480 * 4
    assert first_at is not None
    assert after - 1 < first_at < after
