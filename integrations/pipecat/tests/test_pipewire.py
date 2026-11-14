"""The transport against a real PipeWire (Linux): what a pipeline says is what a
recording of the bot's microphone hears. Opt-in, as the SDK's own PipeWire
tests are: `VIADUCT_PIPEWIRE=1 pytest tests/test_pipewire.py`."""

import array
import asyncio
import itertools
import math
import os
import sys
import tempfile
from pathlib import Path

import pytest
import viaduct
from pipecat.frames.frames import OutputAudioRawFrame
from pipecat.tests.utils import SleepFrame, run_test

from pipecat_viaduct import ViaductTransport, ViaductTransportParams

pytestmark = pytest.mark.skipif(
    sys.platform != "linux" or os.environ.get("VIADUCT_PIPEWIRE") != "1",
    reason="needs a running PipeWire with pw-record; set VIADUCT_PIPEWIRE=1",
)


def tone(seconds: float, rate: int = 24_000) -> bytes:
    """A 440 Hz tone at amplitude 0.5, 16-bit mono."""
    n = round(seconds * rate)
    return array.array(
        "h", (round(16_383 * math.sin(2 * math.pi * 440 * i / rate)) for i in range(n))
    ).tobytes()


def wav_samples(path: Path) -> list[float]:
    """The float samples of pw-record's WAV file (its `data` chunk)."""
    data = path.read_bytes()
    at = 12
    while at + 8 <= len(data):
        size = int.from_bytes(data[at + 4 : at + 8], "little")
        if data[at : at + 4] == b"data":
            return array.array("f", data[at + 8 : at + 8 + size - size % 4]).tolist()
        at += 8 + size + (size & 1)
    return []


async def test_a_recording_of_the_bots_microphone_hears_what_the_pipeline_says() -> None:
    name = f"Pipecat Test Mic {os.getpid()}"
    transport = ViaductTransport(
        ViaductTransportParams(
            audio_out_enabled=True,
            audio_out_sample_rate=24_000,
            audio_out_end_silence_secs=0,
            mic_name=name,
        )
    )
    node = "viaduct." + viaduct.stable_id("microphone", name)
    with tempfile.TemporaryDirectory() as folder:
        wav = Path(folder) / "heard.wav"
        recorder = await asyncio.create_subprocess_exec(
            *["pw-record", "--target", node, "--rate", "48000", "--channels", "1"],
            *["--format", "f32", str(wav)],
        )
        try:
            await asyncio.sleep(0.5)  # the recorder links up
            speech = OutputAudioRawFrame(audio=tone(2.0), sample_rate=24_000, num_channels=1)
            await run_test(transport.output(), frames_to_send=[speech, SleepFrame(sleep=0.3)])
        finally:
            recorder.terminate()
            await asyncio.wait_for(recorder.wait(), timeout=5)
        heard = wav_samples(wav)

    loud = [s for s in heard if abs(s) > 0.05]
    assert len(loud) > 48_000, f"heard {len(heard)} samples, {len(loud)} loud"
    steady = loud[4_800:-4_800]
    rms = math.sqrt(sum(s * s for s in steady) / len(steady))
    rising = sum(1 for a, b in itertools.pairwise(steady) if a < 0 <= b)
    frequency = rising / (len(steady) / 48_000)
    assert abs(rms - 0.354) < 0.03, rms
    assert abs(frequency - 440) < 10, frequency
