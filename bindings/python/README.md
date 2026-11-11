# viaduct-audio

Virtual microphones and speakers that every app sees as real audio devices,
from Python. Your code writes to a microphone, and Zoom, Teams, Discord or a
browser records it. Other apps play to a speaker, and your code reads it.

```bash
pip install viaduct-audio
```

It needs Python 3.14 or newer and has no dependencies. On Windows, the Viaduct
driver must be installed once, as administrator, with `viaduct-cli install`. On
Linux it uses PipeWire, which most desktops already run.

## Quickstart

```python
import viaduct

# 24 kHz 16-bit PCM in, the way most speech models send it.
with viaduct.Microphone("Agent Mic", format=viaduct.Format(24_000, "pcm16")) as mic:
    mic.write(pcm_audio)  # waits as the audio plays; nothing is dropped
    mic.wait_for_playout()
```

```python
with viaduct.Speaker("Agent Speaker", format=viaduct.Format(16_000, "pcm16")) as speaker:
    for chunk in speaker:  # what other apps play, as it plays
        transcriber.feed(chunk)
```

The devices exist while the objects are open. Leaving the `with` block, or the
process ending, removes them.

With asyncio, use `viaduct.aio`. Waiting happens in a worker thread, so the
event loop keeps running, and a call can be cancelled:

```python
from viaduct.aio import AsyncMicrophone

async with AsyncMicrophone("Agent Mic", format=viaduct.Format(24_000, "pcm16")) as mic:
    await mic.write(pcm_audio)
    async with asyncio.timeout(5):
        await mic.wait_for_playout()
```

## Voice agents: barge-in

A speech model sends its reply much faster than real time. When someone talks
over the agent, it has to stop at once, and the model needs to know how much of
its reply was actually heard. A `Playout` does both:

```python
voice = mic.playout()  # takes the model's format from the microphone
voice.push(audio_delta)  # returns at once; plays at the listeners' pace
cut = voice.interrupt()  # someone spoke: stop now
cut.heard_ms  # how much was heard: OpenAI's audio_end_ms
```

[`examples/barge_in.py`](examples/barge_in.py) is a whole agent in a meeting:
OpenAI's realtime model through Vercel's AI Gateway, with barge-in.

## What it takes care of

| The usual problem | What Viaduct does |
| --- | --- |
| Playing audio needs an external player (mpv, ffplay) or a sound library | The devices are built in, with no dependencies |
| The model's format isn't the device's | Pass `format=`; it converts (8 to 48 kHz, f32, pcm16, μ-law; mono or stereo) and keeps state between writes, so chunk edges don't click |
| A blocking `write` freezes async code | `viaduct.aio`; the sync calls also release the GIL while they wait |
| Picking a device by list index breaks when devices change | You create the device and give it its name, and `stable_id()` gives the same ID every time |
| Interrupting means killing the player | `Playout.interrupt()` stops within about 20 ms and says how much was heard |
| Audio goes missing without a word | Lost audio is logged on the `viaduct` logger; `strict=True` raises `AudioLost`; `stats` has the counts |

## Errors

Every error is a `viaduct.Error`, with one subclass per cause: `DuplicateName`,
`DriverNotInstalled`, `UnsupportedFormat`, and so on. Where there's a fix, the
message says what it is.

## Developing

```bash
uv sync                      # builds the extension, with the test backend
uv run --no-sync pytest
uv run --no-sync ruff check . && uv run --no-sync ruff format --check .
uv run --no-sync ty check
```

The tests use a test backend instead of the driver, so they run anywhere. The
audio rules themselves are tested in the Rust SDK; these tests check that Python
reaches them correctly.
