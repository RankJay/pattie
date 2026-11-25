// close() and waits: nothing waits forever, and nothing starves the app.

import assert from 'node:assert/strict';
import { pbkdf2 } from 'node:crypto';
import { test } from 'node:test';
import { promisify } from 'node:util';

import { Microphone, PattieError } from '@pattie/pattie';
import { BACKEND, FakeBackend } from '#testing';

const isClosed = (error: unknown) => error instanceof PattieError && error.code === 'CLOSED';

test('close() ends a write nobody records at once, and it resolves with what it wrote', async () => {
    const mic = new Microphone('Agent Mic', { [BACKEND]: new FakeBackend() });
    // Nobody pulls: the lead fills, then the write waits for room forever.
    const writing = mic.write(new Float32Array(48_000));
    await new Promise((resolve) => setTimeout(resolve, 50));

    const closedAt = performance.now();
    mic.close();
    const written = await writing;

    assert.ok(written > 0 && written <= 2_400, `${written} written`);
    assert.ok(performance.now() - closedAt < 50, 'the write ended at its piece, not at once');
});

test('after close() the name is free, and calls are CLOSED', async () => {
    const backend = new FakeBackend();
    const mic = new Microphone('Agent Mic', { [BACKEND]: backend });
    mic.close();
    mic.close(); // twice is fine

    using _again = new Microphone('Agent Mic', { [BACKEND]: backend });
    assert.throws(() => mic.writeNowait(new Float32Array(480)), isClosed);
    await assert.rejects(mic.write(new Float32Array(480)), isClosed);
});

test('an AbortSignal ends a write within a piece, and the microphone stays usable', async () => {
    const backend = new FakeBackend();
    using mic = new Microphone('Agent Mic', { [BACKEND]: backend });
    const startedAt = performance.now();

    // Nobody pulls, so only the signal ends it.
    await assert.rejects(
        mic.write(new Float32Array(48_000), { signal: AbortSignal.timeout(50) }),
        (error: unknown) => error instanceof DOMException && error.name === 'TimeoutError',
    );

    const took = performance.now() - startedAt;
    assert.ok(took < 300, `aborted after ${took.toFixed(0)} ms`);
    backend.systemRead(mic.id, 4_096);
    assert.ok(mic.writeNowait(new Float32Array(480)) > 0);
});

test('an AbortSignal already aborted rejects without writing', async () => {
    using mic = new Microphone('Agent Mic', { [BACKEND]: new FakeBackend() });
    const controller = new AbortController();
    controller.abort();

    await assert.rejects(
        mic.write(new Float32Array(480), { signal: controller.signal }),
        (error: unknown) => error instanceof DOMException && error.name === 'AbortError',
    );
    assert.equal(mic.stats.bufferedFrames, 0);
});

test('waits on many Devices leave libuv pool free for the app', async () => {
    const backend = new FakeBackend();
    // More Devices waiting than libuv's pool has threads (4).
    const mics = Array.from(
        { length: 6 },
        (_, i) => new Microphone(`Agent Mic ${i}`, { [BACKEND]: backend }),
    );
    const writes = mics.map((mic) => mic.write(new Float32Array(48_000)));
    await new Promise((resolve) => setTimeout(resolve, 20));

    // pbkdf2 runs on libuv's pool: it would queue behind the waits if they used it.
    const startedAt = performance.now();
    await promisify(pbkdf2)('secret', 'salt', 1, 32, 'sha256');
    const took = performance.now() - startedAt;

    for (const mic of mics) mic.close();
    await Promise.all(writes);
    assert.ok(took < 100, `pbkdf2 took ${took.toFixed(0)} ms`);
});
