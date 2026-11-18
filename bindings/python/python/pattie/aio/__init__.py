"""Pattie for asyncio: the same Devices, with the calls that wait made awaitable.

Waiting happens on Pattie's own worker threads (a few per object, made as
needed), so the event loop keeps running, and an app that keeps its default
thread pool busy can't starve the audio. Each wait in those threads lasts at
most 100 ms, so cancelling a call stops it within about that long, and closing
never waits on a stuck call. To give up after a while, use ``asyncio.timeout``::

    async with AsyncMicrophone("Agent Mic", format=AudioFormat(24_000, "pcm16")) as mic:
        await mic.write(audio)
        async with asyncio.timeout(5):
            await mic.wait_for_playout()
"""

from pattie.aio.microphone import AsyncMicrophone
from pattie.aio.playout import AsyncPlayout
from pattie.aio.speaker import AsyncSpeaker

__all__ = ["AsyncMicrophone", "AsyncPlayout", "AsyncSpeaker"]
