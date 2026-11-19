# pattie (Python)

Virtual microphones and speakers that every app sees as real audio devices,
from Python. Your code writes to a microphone, and Zoom, Teams, Discord or a
browser records it. Other apps play to a speaker, and your code reads it.

```bash
pip install pattie
```

It needs Python 3.14 or newer and has no dependencies. On Windows, the Pattie
driver must be installed once, as administrator, with `pattie-cli install`. On
Linux it uses PipeWire, which most desktops already run.

## Quickstart

```python
import pattie

# 24 kHz 16-bit PCM in, the way most speech models send it.
with pattie.Microphone("Agent Mic", format=pattie.AudioFormat(24_000, "pcm16")) as mic:
    mic.write(pcm_audio)  # waits as the audio plays; nothing is dropped
    mic.wait_for_playout()
```

```python
with pattie.Speaker("Agent Speaker", format=pattie.AudioFormat(16_000, "pcm16")) as speaker:
    for chunk in speaker:  # what other apps play, as it plays
        transcriber.feed(chunk)
```

The devices exist while the objects are open. Leaving the `with` block, or the
process ending, removes them.
A `close()` from another thread ends a call that's waiting (it returns what it
managed within about 100 ms), and Ctrl+C interrupts one.

With asyncio, use `pattie.aio`. Waiting happens in a worker thread, so the
event loop keeps running, and a call can be cancelled:

```python
from pattie.aio import AsyncMicrophone

async with AsyncMicrophone("Agent Mic", format=pattie.AudioFormat(24_000, "pcm16")) as mic:
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
voice.push(audio_delta, reply=item_id)  # returns at once; plays at the listeners' pace
cut = voice.interrupt()  # someone spoke: stop now
cut.reply, cut.heard_ms  # which reply, and how much was heard: OpenAI's audio_end_ms
```

Name each reply (`reply=`, OpenAI's item id). A reply then stays one reply
however its audio arrives: a network stall doesn't start a new one, and the next
reply's audio, pushed while this one still plays, doesn't join it. Without a
name, a reply ends whenever all of its audio has played.

[`examples/barge_in.py`](examples/barge_in.py) is a whole agent in a meeting:
OpenAI's realtime model through Vercel's AI Gateway, with barge-in.

## Relaying a live source

Audio relayed in real time from something with its own clock, such as a phone
call or a capture card, arrives a little faster or slower than the device plays
it. Over hours that adds up to seconds of delay, or gaps. A `LiveWriter` holds
it at about 40 ms and keeps it there by resampling a few parts per million:

```python
relay = mic.live_writer(pattie.AudioFormat(16_000, "pcm16"))
relay.push(chunk)  # as each chunk arrives; never waits
relay.stats.correction_ppm  # how far off the source's clock is
```

## Timestamps

Times are `time.perf_counter()` seconds, so they compare directly with your own.

```python
data, played_at = speaker.read_timed(timeout=1)  # when its first frame played
mic.next_play_time  # when the next frame written will be heard
mic.clock  # ClockPoint(frames, at): how far listeners had got, and when
```

`read_timed` gives an echo canceller its reference; `next_play_time` keeps audio
in step with video.

## What it takes care of

| The usual problem | What Pattie does |
| --- | --- |
| Playing audio needs an external player (mpv, ffplay) or a sound library | The devices are built in, with no dependencies |
| The model's format isn't the device's | Pass `format=`; it converts (8 to 96 kHz, f32, pcm16, μ-law; 1 to 8 channels, to or from mono) and keeps state between writes, so chunk edges don't click |
| The device should be stereo, 44.1 or 96 kHz | `Microphone(name, rate=44_100, channels=2)`: the device itself runs in that format (1 to 8 channels; 44.1, 48 or 96 kHz), and other apps' audio is converted by the system |
| A blocking `write` freezes async code | `pattie.aio`; the sync calls also release the GIL while they wait |
| Picking a device by list index breaks when devices change | You create the device and give it its name, and `stable_id()` gives the same ID every time for the same name and format |
| Interrupting means killing the player | `Playout.interrupt()` stops within about 20 ms and says how much was heard |
| A relayed stream drifts: seconds of delay after a few hours | `live_writer()` holds its latency with drift compensation |
| Audio goes missing without a word | Lost audio is logged on the `pattie` logger; `strict=True` raises `AudioLost`; `stats` has the counts |

## Errors

Every error is a `pattie.Error`, with one subclass per cause: `DuplicateName`,
`DriverNotInstalled`, `UnsupportedFormat`, and so on. Where there's a fix, the
message says what it is. `DriverRemoved` means the driver was removed or is
being updated while the Device was live (Windows): create the Device again.

## Developing

```bash
uv sync                      # builds the extension, with the Fake backend
uv run --no-sync pytest
uv run --no-sync ruff check . && uv run --no-sync ruff format --check .
uv run --no-sync ty check
```

The tests use a Fake backend instead of the driver, so they run anywhere. The
audio rules themselves are tested in the Rust SDK; these tests check that Python
reaches them correctly.
