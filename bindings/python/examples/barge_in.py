"""A voice agent in a meeting, with barge-in.

OpenAI's realtime model, reached through Vercel's AI Gateway, speaks through
"Agent Mic" and listens through "Agent Speaker". Talk over it and it stops
mid-sentence, then answers what you said, knowing which of its words you heard.

    pip install pattie websockets
    AI_GATEWAY_API_KEY=vck_... python barge_in.py

Then, in Zoom (or Teams, Meet, ...), pick microphone "Agent Mic" and speaker
"Agent Speaker". Ctrl+C ends it, and both devices disappear.

How it works:
- The speaker hands over what the meeting plays already as the 24 kHz 16-bit
  PCM the model takes, so it goes to the model as it is.
- The model sends its audio much faster than real time. A Playout plays it at
  the meeting's pace.
- Each delta is pushed under its item id, so the playout knows where each reply
  begins and ends, whatever the network does.
- When the model hears someone start talking (``speech-started``), the playout
  is interrupted at once, and ``conversation-item-truncate`` tells the model how
  much of the reply being heard was heard, so it forgets the rest.
"""

import asyncio
import base64
import json
import os

import websockets
from websockets.typing import Subprotocol

import pattie
from pattie.aio import AsyncMicrophone, AsyncSpeaker

URL = "wss://ai-gateway.vercel.sh/v4/ai/realtime-model?ai-model-id=openai/gpt-realtime-2"
# The model's audio, both ways: 16-bit PCM, 24 kHz, mono.
MODEL_AUDIO = pattie.AudioFormat(24_000, "pcm16")


async def main() -> None:
    key = os.environ["AI_GATEWAY_API_KEY"]
    async with (
        AsyncMicrophone("Agent Mic", format=MODEL_AUDIO) as mic,
        AsyncSpeaker("Agent Speaker", format=MODEL_AUDIO) as speaker,
        # The API key goes in a second subprotocol: browsers can't set headers.
        websockets.connect(
            URL,
            subprotocols=[
                Subprotocol("ai-gateway-realtime.v1"),
                Subprotocol(f"ai-gateway-auth.{key}"),
            ],
        ) as socket,
    ):
        voice = mic.playout()
        print('"Agent Mic" and "Agent Speaker" are ready: pick them in your meeting app.')
        # The Gateway closes a session whose first message doesn't come within 30 s.
        await socket.send(
            json.dumps(
                {
                    "type": "session-update",
                    "config": {
                        "instructions": "You are in a meeting. Keep answers short.",
                        "voice": "marin",
                        "outputModalities": ["audio"],
                        "turnDetection": {"type": "server-vad"},
                    },
                }
            )
        )

        async def listen() -> None:
            async for chunk in speaker:
                audio = base64.b64encode(chunk).decode()
                await socket.send(json.dumps({"type": "input-audio-append", "audio": audio}))

        async def talk() -> None:
            async for message in socket:
                event = json.loads(message)
                match event["type"]:
                    case "audio-delta":
                        voice.push(base64.b64decode(event["delta"]), reply=event.get("itemId"))
                    case "speech-started":
                        cut = voice.interrupt()  # says which reply was being heard
                        if cut.reply and cut.heard_frames:
                            await socket.send(
                                json.dumps(
                                    {
                                        "type": "conversation-item-truncate",
                                        "itemId": cut.reply,
                                        "contentIndex": 0,
                                        "audioEndMs": cut.heard_ms,
                                    }
                                )
                            )
                            print(f"Interrupted after {cut.heard_ms} ms of the reply.")
                    case "error":
                        print("AI Gateway error:", event.get("message"), event.get("code"))
                    case "session-closed":
                        print("The Gateway closed the session:", event.get("reason"))
                        return

        listener = asyncio.create_task(listen())
        try:
            await talk()
        finally:
            listener.cancel()  # the session ended: stop listening too


if __name__ == "__main__":
    asyncio.run(main())
