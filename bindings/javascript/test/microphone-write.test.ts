// Writing a Microphone's audio: write(), writeNowait(), and what they take.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { Microphone, PattieError } from '@pattie/pattie';
import { BACKEND, FakeBackend } from '#testing';

import { nextWarning, pullInBackground, ramp } from './system.ts';

test('write resolves once the system has taken it all, without blocking the event loop', async () => {
    const backend = new FakeBackend();
    using mic = new Microphone('Agent Mic', { [BACKEND]: backend });
    // Takes only what's buffered: a late write would otherwise hear Gap first.
    using system = pullInBackground(backend, mic.id, () => mic.stats.bufferedFrames);
    const audio = ramp(9_600); // 200 ms

    const written = await mic.write(audio);
    await mic.waitForPlayout();

    assert.equal(written, 9_600);
    // The system's pulls ran on this event loop while the write waited.
    assert.deepEqual(system.heard.slice(0, 9_600), Array.from(audio));
});

test("16-bit PCM at 24 kHz, as voice APIs send it, is converted to the microphone's own", async () => {
    const backend = new FakeBackend();
    const format = { rate: 24_000, encoding: 'pcm16' } as const;
    using mic = new Microphone('Agent Mic', { format, [BACKEND]: backend });
    using _system = pullInBackground(backend, mic.id);

    const written = await mic.write(new Int16Array(2_400).fill(8_192)); // 100 ms

    assert.equal(written, 4_800); // 100 ms at 48 kHz
    assert.deepEqual(mic.format, { rate: 24_000, encoding: 'pcm16', channels: 1 });
    assert.deepEqual([mic.rate, mic.channels], [48_000, 1]);
});

test('a Buffer over part of a larger ArrayBuffer is read within its own bytes', () => {
    const backend = new FakeBackend();
    using mic = new Microphone('Agent Mic', { [BACKEND]: backend });
    // As Node's Buffer pool hands them out: 8 bytes, 16 bytes into 64.
    const backing = new ArrayBuffer(64);
    new Float32Array(backing).fill(0.9);
    new Float32Array(backing, 16, 2).set([0.25, 0.5]);
    const view = Buffer.from(backing, 16, 8);

    assert.equal(mic.writeNowait(view), 2);
    assert.deepEqual(backend.systemRead(mic.id, 4), new Float32Array([0.25, 0.5, 0, 0]));
});

test('a Float32Array for 16-bit PCM is a TypeError, before anything is written', async () => {
    using mic = new Microphone('Agent Mic', {
        format: { rate: 24_000, encoding: 'pcm16' },
        [BACKEND]: new FakeBackend(),
    });

    await assert.rejects(mic.write(new Float32Array(480)), TypeError);
    assert.throws(() => mic.writeNowait(new Uint32Array(480)), TypeError);
    assert.equal(mic.stats.bufferedFrames, 0);
});

test('playedFrames and playedMs count what the system has taken', () => {
    const backend = new FakeBackend();
    using mic = new Microphone('Agent Mic', { [BACKEND]: backend });
    mic.writeNowait(new Float32Array(2_400));

    backend.systemRead(mic.id, 1_200);

    assert.deepEqual([mic.playedFrames, mic.playedMs], [1_200, 25]);
});

test('closed says whether close() was called, and await using closes it', async () => {
    const backend = new FakeBackend();
    let kept: Microphone;
    {
        await using mic = new Microphone('Agent Mic', { [BACKEND]: backend });
        kept = mic;
        assert.equal(mic.closed, false);
    }
    assert.equal(kept.closed, true);
});

test('writeNowait writes what fits now, and warns PATTIE_AUDIO_LOST about the rest', async () => {
    using mic = new Microphone('Agent Mic', { [BACKEND]: new FakeBackend() });
    const warned = nextWarning('writeNowait()');

    // Nobody pulls: a 4,096-frame Ring takes 4,096 of the 48,000 frames.
    const written = mic.writeNowait(new Float32Array(48_000));

    assert.equal(written, 4_096);
    const warning = await warned;
    assert.equal(warning.code, 'PATTIE_AUDIO_LOST');
    assert.match(warning.message, /43904 frames of Overflow/);
});

test('with strict, writeNowait throws AUDIO_LOST instead of warning', () => {
    using mic = new Microphone('Agent Mic', { strict: true, [BACKEND]: new FakeBackend() });

    assert.throws(
        () => mic.writeNowait(new Float32Array(48_000)),
        (error: unknown) => error instanceof PattieError && error.code === 'AUDIO_LOST',
    );
});
