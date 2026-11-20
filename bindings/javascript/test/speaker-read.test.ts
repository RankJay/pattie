// Reading a Speaker's audio: read(), iteration, and what they return.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { setTimeout as sleep } from 'node:timers/promises';

import { Speaker } from '@pattie/pattie';
import { FakeBackend } from '@pattie/pattie/_testing';

import { ramp } from './system.ts';

test('read resolves with what the system played, as soon as it plays', async () => {
    const backend = new FakeBackend();
    using speaker = new Speaker('Agent Speaker', { _backend: backend });
    const played = ramp(480);
    setTimeout(() => backend.systemWrite(speaker.id, played), 20);

    const heard = await speaker.read();

    assert.ok(heard instanceof Float32Array);
    assert.deepEqual(heard, played);
});

test('a speaker with a 16-bit PCM format returns an Int16Array at its rate', async () => {
    const backend = new FakeBackend();
    using speaker = new Speaker('Agent Speaker', {
        format: { rate: 16_000, encoding: 'pcm16' },
        _backend: backend,
    });
    backend.systemWrite(speaker.id, new Float32Array(2_400).fill(0.5)); // 50 ms

    const heard = await speaker.read();

    assert.ok(heard instanceof Int16Array);
    // 50 ms at 16 kHz, less what the resampler holds back for its next read.
    assert.ok(heard.length > 700 && heard.length <= 800, `${heard.length} samples`);
    assert.deepEqual([speaker.rate, speaker.channels], [48_000, 1]);
});

test('for await yields what is played, and ends when the speaker is closed', async () => {
    const backend = new FakeBackend();
    const speaker = new Speaker('Agent Speaker', { _backend: backend });
    backend.systemWrite(speaker.id, ramp(480));
    setTimeout(() => speaker.close(), 50);

    const chunks: Float32Array[] = [];
    for await (const chunk of speaker) chunks.push(chunk);

    assert.equal(chunks.length, 1);
    assert.deepEqual(chunks[0], ramp(480));
});

test('close() ends a read nobody plays to at once, with nothing', async () => {
    const speaker = new Speaker('Agent Speaker', { _backend: new FakeBackend() });
    const reading = speaker.read();
    await sleep(20);

    const closedAt = performance.now();
    speaker.close();
    const heard = await reading;

    assert.equal(heard.length, 0);
    assert.ok(performance.now() - closedAt < 50);
});

test('an AbortSignal ends a read', async () => {
    using speaker = new Speaker('Agent Speaker', { _backend: new FakeBackend() });

    await assert.rejects(
        speaker.read({ signal: AbortSignal.timeout(30) }),
        (error: unknown) => error instanceof DOMException && error.name === 'TimeoutError',
    );
});
