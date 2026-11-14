# pipecat-viaduct

A [Pipecat](https://github.com/pipecat-ai/pipecat) transport that gives a bot its
own microphone and speaker. Zoom, Teams, Meet, Discord and OBS list them as
ordinary devices, "Microphone (Agent Mic)" and "Speakers (Agent Speaker)", with no
virtual cable to install and no window to share.

```python
from pipecat_viaduct import ViaductTransport, ViaductTransportParams

transport = ViaductTransport(
    ViaductTransportParams(audio_in_enabled=True, audio_out_enabled=True)
)
pipeline = Pipeline([
    transport.input(),    # what the meeting plays to "Agent Speaker"
    stt, user_aggregator, llm, tts,
    transport.output(),   # what the bot says, out through "Agent Mic"
    assistant_aggregator,
])
```

The Devices appear as soon as the transport is created, so a meeting app can be
pointed at them before the bot starts. They go away when the pipeline ends, and
come back under the same names, as the same devices, the next time it runs.

## What it does for you

| | |
| --- | --- |
| Formats | The pipeline keeps its own rates (16 kHz in and 24 kHz out, say); Viaduct converts to and from the Devices' 48 kHz |
| Interruptions | Pipecat's own: the device holds only ~20 ms beyond the chunk being written, so a barge-in cuts within about 60 ms (set `audio_out_10ms_chunks=2` for less) |
| Speaking events | Writes take as long as the audio does, so Pipecat's bot-started / bot-stopped speaking events and its clock stay true |
| Turn ends | While nothing plays to the speaker, the input sends silence in 20 ms frames, so voice activity detection always sees a turn end |
| Lost audio | Logged as a warning (logger `viaduct`), or raised as `viaduct.AudioLost` with `strict=True` |

## Parameters

Everything in Pipecat's `TransportParams`, plus:

| Parameter | Default | |
| --- | --- | --- |
| `mic_name` | `"Agent Mic"` | The microphone's name in meeting apps |
| `speaker_name` | `"Agent Speaker"` | The speaker's name |
| `device_rate` | `48_000` | The Devices' rate: 44,100, 48,000 or 96,000 |
| `device_channels` | `1` | The Devices' channels, 1 to 8 |
| `strict` | `False` | Raise on lost audio instead of logging it |

Only the enabled sides get a Device (`audio_in_enabled`, `audio_out_enabled`).
`transport.microphone` and `transport.speaker` are the `viaduct.Microphone` and
`viaduct.Speaker`, for their stats and clocks.

## Requirements

- Python 3.14 and `pipecat-ai` 1.16 or newer (1.x).
- Windows 11 with the Viaduct driver installed, or Linux with PipeWire (the
  `viaduct-audio` package's requirements).

## Try it

`examples/echo_bot.py` needs no API keys: it says back what it hears a second
later. Run it, pick "Agent Mic" and "Agent Speaker" in a meeting app's test call,
and talk.

```bash
uv run examples/echo_bot.py
```

## Development

```bash
uv sync                 # builds the local viaduct binding with its test backend
uv run pytest -q        # the transport against the SDK's test backend
VIADUCT_PIPEWIRE=1 uv run pytest -q tests/test_pipewire.py   # Linux, real PipeWire
```
