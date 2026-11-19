# pattie (Node.js)

Virtual microphones and speakers that every app sees as real audio devices,
from Node.js and Electron. Your code writes to a microphone, and Zoom, Teams,
Discord or a browser records it. Other apps play to a speaker, and your code
reads it.

```bash
npm install pattie
```

It needs Node.js 22.12 or newer, or Electron, and has no dependencies. The
examples use `using`, which Node 24 runs as it is; on Node 22, compile with
TypeScript (it turns `using` into try/finally) or call `close()` yourself. A
prebuilt binary is installed for your platform (Windows x64, Linux x64 and
arm64): no compiler, no node-gyp, no `electron-rebuild`. On Windows, the Pattie
driver must be installed once, as administrator, with `pattie-cli install`. On
Linux it uses PipeWire, which most desktops already run.

## Quickstart

```ts
import { Microphone, Speaker } from 'pattie';

// 24 kHz 16-bit PCM in, the way most speech models send it.
using mic = new Microphone('Agent Mic', { format: { rate: 24_000, encoding: 'pcm16' } });
await mic.write(pcmAudio); // an Int16Array, a Buffer, ...: resolves as it plays
await mic.waitForPlayout();
```

```ts
using speaker = new Speaker('Agent Speaker', { format: { rate: 16_000, encoding: 'pcm16' } });
for await (const chunk of speaker) {
    transcriber.feed(chunk); // an Int16Array of what other apps play, as it plays
}
```

The devices exist while the objects are open. Leaving the `using` block,
`close()`, or the process ending, removes them.

Nothing blocks the event loop. A call that waits returns a Promise and waits on
a thread of the Device's own (not libuv's pool, so your file and DNS calls go
on). `close()` ends a wait at once; it resolves with what it managed. Every
call that waits takes an `AbortSignal`:

```ts
await mic.waitForPlayout({ signal: AbortSignal.timeout(5_000) });
```

## Voice agents: barge-in

A speech model sends its reply much faster than real time. When someone talks
over the agent, it has to stop at once, and the model needs to know how much of
its reply was actually heard. A `Playout` does both:

```ts
using voice = mic.playout(); // takes the model's format from the microphone
voice.push(Buffer.from(delta, 'base64'), { reply: itemId }); // returns at once
const cut = voice.interrupt(); // someone spoke: stop now
cut.reply, cut.heardMs; // which reply, and how much was heard: OpenAI's audio_end_ms
```

[examples/barge-in.ts](examples/barge-in.ts) is the whole agent: OpenAI's
realtime model through Vercel's AI Gateway, in a meeting, with barge-in.

## Audio in, audio out

- **Formats** are plain objects: `{ rate, encoding, channels }`, with `encoding`
  `'f32'`, `'pcm16'` or `'ulaw'` and rates from 8 to 96 kHz. A Device converts
  to and from its own (`DEVICE_FORMAT`, 48 kHz mono float) as it goes.
- **Writes** take a `Float32Array` (f32), an `Int16Array` (pcm16), or bytes
  (`Buffer`, `Uint8Array`, `ArrayBuffer`). A `Buffer` from Node's pool is read
  within its own bytes. An array that doesn't match the format is a `TypeError`.
- **Reads** return the array that matches the format: `Float32Array`,
  `Int16Array`, or `Uint8Array` for μ-law. `readTimed()` also says when the
  first frame was played, in `performance.now()` milliseconds.
- `writeNowait()` writes what fits now and drops the rest. Lost audio is a
  `PATTIE_AUDIO_LOST` warning (`process.emitWarning`, at most once a second),
  or a thrown `AUDIO_LOST` with `{ strict: true }`.
- `mic.liveWriter()` is for a live source with its own clock (a network call):
  pushes never wait, and a jitter buffer resampled a few ppm stays at about
  40 ms for hours.
- `new Converter(from, to)` converts audio that isn't a Device's.

## Errors

Every error is a `PattieError` with a `code`, and its message says the fix:

| `code` | When |
| --- | --- |
| `DRIVER_NOT_INSTALLED` | Windows: install the driver with `pattie-cli install` |
| `AUDIO_SERVER_UNAVAILABLE` | Linux: PipeWire isn't running (or PulseAudio serves audio) |
| `DUPLICATE_NAME` | a Device of this kind already has the name |
| `TOO_MANY_DEVICES` | at most 8 at a time |
| `ALREADY_TAKEN` | a Microphone has one writer: `write()`, a playout or a live writer |
| `UNSUPPORTED_FORMAT` | a rate or channel mapping the converter can't take |
| `AUDIO_LOST` | audio was dropped or ran out, with `strict` |
| `CLOSED` | the Device was closed |
| `DRIVER_REMOVED` | the driver was updated while the Device was live: create it again |

and `ACCESS_DENIED`, `NO_BACKEND`, `VERSION_MISMATCH`, `BAD_BUFFER_SIZE`,
`BAD_NAME`, `WRONG_KIND`, `SYSTEM_REFUSED`, `PROTOCOL_ERROR`, `OS_ERROR`: the
same errors as the Python binding's exceptions (ADR 0019).

## Working on it

```bash
pnpm install
pnpm build:dev     # the native module with the SDK's Fake backend, and pattie/_testing
pnpm test          # node:test against the Fake backend: no driver needed
pnpm typecheck && pnpm lint && pnpm format:check
pnpm build && pnpm test:release   # a release build, packed and installed as users get it
```

`src/` is the Rust (napi-rs) over the SDK; `lib/` is the TypeScript layer,
which holds only JavaScript idioms (`using`, `for await`, `AbortSignal`, the
error class). Every audio rule lives in the SDK. The design is ADR 0019's
amendment of 2026-11-20.
