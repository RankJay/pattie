// A Microphone's Playout: push replies as fast as a model sends them,
// interrupt() the moment someone talks over it (barge-in).

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { setTimeout as sleep } from 'node:timers/promises';

import { Microphone, PattieError } from 'pattie';
import { FakeBackend } from 'pattie/_testing';

import { pullInBackground } from './system.ts';

const isCode = (code: string) => (error: unknown) =>
    error instanceof PattieError && error.code === code;

test('push queues a reply at once, and interrupt() says how much of it was heard', async () => {
    const backend = new FakeBackend();
    using mic = new Microphone('Agent Mic', { _backend: backend });
    using playout = mic.playout();

    assert.equal(playout.push(new Float32Array(48_000), { reply: 'greeting' }), 48_000); // 1 s
    using _system = pullInBackground(backend, mic.id);
    await sleep(100);
    const interrupted = playout.interrupt();

    assert.equal(interrupted.reply, 'greeting');
    assert.ok(interrupted.heardFrames > 0 && interrupted.heardFrames < 24_000);
    // What's in the Ring already (at most the lead, about 20 ms) still plays.
    const stillPlaying = 48_000 - interrupted.heardFrames - interrupted.discardedFrames;
    assert.ok(stillPlaying >= 0 && stillPlaying <= 2_400, `${stillPlaying} frames still playing`);
    assert.equal(interrupted.heardMs, Math.floor((interrupted.heardFrames * 1_000) / 48_000));
    assert.ok(Object.isFrozen(interrupted));
    assert.equal(playout.queuedFrames, 0);
});

test('waitIdle resolves once everything pushed has been heard', async () => {
    const backend = new FakeBackend();
    using mic = new Microphone('Agent Mic', {
        format: { rate: 24_000, encoding: 'pcm16' },
        _backend: backend,
    });
    using playout = mic.playout();
    playout.push(new Int16Array(2_400)); // 100 ms
    using _system = pullInBackground(backend, mic.id);

    assert.equal(await playout.waitIdle(), true);
});

test('a microphone has one writer: write() or a playout, not both', async () => {
    using mic = new Microphone('Agent Mic', { _backend: new FakeBackend() });
    mic.writeNowait(new Float32Array(480));

    assert.throws(() => mic.playout(), isCode('ALREADY_TAKEN'));
});

test('closing the microphone ends its playout and a waitIdle in progress', async () => {
    const mic = new Microphone('Agent Mic', { _backend: new FakeBackend() });
    const playout = mic.playout();
    playout.push(new Float32Array(48_000));
    const waiting = playout.waitIdle();
    await sleep(20);

    mic.close();

    assert.equal(await waiting, false);
    assert.throws(() => playout.push(new Float32Array(480)), isCode('CLOSED'));
});
