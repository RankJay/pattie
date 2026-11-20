// A voice agent in a meeting, with barge-in.
//
// OpenAI's realtime model, reached through Vercel's AI Gateway, speaks through
// "Agent Mic" and listens through "Agent Speaker". Talk over it and it stops
// mid-sentence, then answers what you said, knowing which of its words you heard.
//
//     npm install pattie
//     AI_GATEWAY_API_KEY=vck_... node barge-in.ts
//
// Then, in Zoom (or Teams, Meet, ...), pick microphone "Agent Mic" and speaker
// "Agent Speaker". Ctrl+C ends it, and both devices disappear.
//
// How it works (as the Python binding's barge_in.py):
// - The speaker hands over what the meeting plays already as the 24 kHz 16-bit
//   PCM the model takes, so it goes to the model as it is.
// - The model sends its audio much faster than real time. A Playout plays it at
//   the meeting's pace.
// - Each delta is pushed under its item id, so the playout knows where each
//   reply begins and ends, whatever the network does.
// - When the model hears someone start talking (`speech-started`), the playout
//   is interrupted at once, and `conversation-item-truncate` tells the model how
//   much of the reply being heard was heard, so it forgets the rest.

import { type AudioFormat, Microphone, Speaker } from '@pattie/pattie';

const URL = 'wss://ai-gateway.vercel.sh/v4/ai/realtime-model?ai-model-id=openai/gpt-realtime-2';
// The model's audio, both ways: 16-bit PCM, 24 kHz, mono.
const MODEL_AUDIO = { rate: 24_000, encoding: 'pcm16' } as const satisfies AudioFormat;

const key = process.env.AI_GATEWAY_API_KEY;
if (!key) throw new Error('set AI_GATEWAY_API_KEY');

await using mic = new Microphone('Agent Mic', { format: MODEL_AUDIO });
await using speaker = new Speaker('Agent Speaker', { format: MODEL_AUDIO });
using voice = mic.playout();
// The API key goes in a second subprotocol: browsers can't set headers.
const socket = new WebSocket(URL, ['ai-gateway-realtime.v1', `ai-gateway-auth.${key}`]);
const send = (event: object) => socket.send(JSON.stringify(event));
await new Promise((resolve, reject) => {
    socket.addEventListener('open', resolve, { once: true });
    socket.addEventListener('error', reject, { once: true });
});
console.log('"Agent Mic" and "Agent Speaker" are ready: pick them in your meeting app.');

// The Gateway closes a session whose first message doesn't come within 30 s.
send({
    type: 'session-update',
    config: {
        instructions: 'You are in a meeting. Keep answers short.',
        voice: 'marin',
        outputModalities: ['audio'],
        turnDetection: { type: 'server-vad' },
    },
});

// Listen: what the meeting plays, to the model.
const listening = (async () => {
    for await (const chunk of speaker) {
        send({ type: 'input-audio-append', audio: Buffer.from(chunk.buffer).toString('base64') });
    }
})();

// Talk: the model's audio, to the meeting.
const ended = new Promise<void>((resolve) => {
    socket.addEventListener('close', () => resolve());
    socket.addEventListener('message', (message: MessageEvent) => {
        const event = JSON.parse(String(message.data));
        switch (event.type) {
            case 'audio-delta':
                voice.push(Buffer.from(event.delta, 'base64'), { reply: event.itemId });
                break;
            case 'speech-started': {
                const cut = voice.interrupt(); // says which reply was being heard
                if (cut.reply !== null && cut.heardFrames > 0) {
                    send({
                        type: 'conversation-item-truncate',
                        itemId: cut.reply,
                        contentIndex: 0,
                        audioEndMs: cut.heardMs,
                    });
                    console.log(`Interrupted after ${cut.heardMs} ms of the reply.`);
                }
                break;
            }
            case 'error':
                console.log('AI Gateway error:', event.message, event.code);
                break;
            case 'session-closed':
                console.log('The Gateway closed the session:', event.reason);
                socket.close();
                break;
        }
    });
});

process.once('SIGINT', () => socket.close());
await ended;
speaker.close(); // the session ended: stop listening too
await listening;
