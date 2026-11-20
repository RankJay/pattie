// LiveWriter (a live source with its own clock), the Device clock and
// timestamps on performance.now()'s timeline.

import assert from 'node:assert/strict';
import { test } from 'node:test';
import { setTimeout as sleep } from 'node:timers/promises';

import { Converter, Microphone, PattieError, Speaker } from '@pattie/pattie';
import { FakeBackend, sdkError } from '@pattie/pattie/_testing';

import { pullInBackground, ramp } from './system.ts';

test('a LiveWriter takes audio as it arrives, never waits, and reports its jitter buffer', async () => {
    const backend = new FakeBackend();
    using mic = new Microphone('Agent Mic', { _backend: backend });
    using live = mic.liveWriter({ rate: 16_000, encoding: 'pcm16' });
    using _system = pullInBackground(backend, mic.id);

    for (let i = 0; i < 10; i++) {
        assert.equal(live.push(new Int16Array(320).fill(4_096)), 960); // 20 ms
        await sleep(20);
    }

    const stats = live.stats;
    assert.ok(
        stats.queuedFrames >= 0 && stats.queuedFrames < 4_800,
        `${stats.queuedFrames} queued`,
    );
    assert.equal(typeof stats.correctionPpm, 'number');
    assert.ok(Object.isFrozen(stats));
    assert.throws(
        () => mic.playout(),
        (error: unknown) => error instanceof PattieError && error.code === 'ALREADY_TAKEN',
    );
});

test('clock is null until the system runs, then a point on performance.now()', () => {
    const backend = new FakeBackend();
    using mic = new Microphone('Agent Mic', { _backend: backend });
    const [clockBefore, nextBefore] = [mic.clock, mic.nextPlayTime];
    assert.equal(clockBefore, null);
    assert.equal(nextBefore, null);

    mic.writeNowait(new Float32Array(960));
    backend.systemRead(mic.id, 480);
    const now = performance.now();

    const clock = mic.clock;
    assert.equal(clock?.frames, 480);
    assert.ok(Math.abs((clock?.at ?? 0) - now) < 5, `${clock?.at} vs ${now}`);
    // The next frame written plays after the 480 still buffered: 10 ms on.
    const next = mic.nextPlayTime ?? 0;
    assert.ok(Math.abs(next - ((clock?.at ?? 0) + 10)) < 1, `${next}`);
});

test('readTimed says when the first frame it returns was played', async () => {
    const backend = new FakeBackend();
    using speaker = new Speaker('Agent Speaker', { _backend: backend });
    const start = performance.now();
    backend.systemWrite(speaker.id, ramp(480));
    const end = performance.now();

    const { data, firstAt } = await speaker.readTimed();

    assert.deepEqual(data, ramp(480));
    // One 10 ms packet, played by a pass that ended between start and end (as
    // the Python binding's read_timed).
    assert.ok(firstAt !== null, 'no time');
    assert.ok(
        start - 10 - 1 <= firstAt && firstAt <= end - 10 + 1,
        `${firstAt} in ${start}..${end}`,
    );
});

test('a Converter keeps its state between pieces, so a stream converts the same in any pieces', () => {
    const from = { rate: 24_000, encoding: 'pcm16' } as const;
    const to = { rate: 48_000, encoding: 'f32' } as const;
    const audio = Int16Array.from({ length: 2_400 }, (_, i) =>
        Math.round(8_000 * Math.sin(i / 10)),
    );

    const whole = new Converter(from, to).convert(audio);
    const pieces = new Converter(from, to);
    const joined = [
        ...pieces.convert(audio.subarray(0, 1_000)),
        ...pieces.convert(audio.subarray(1_000)),
    ];

    assert.ok(whole instanceof Float32Array);
    assert.deepEqual(joined, Array.from(whole));
});

test('sdkError makes the errors only a real driver gives, with their fixes', () => {
    const error = sdkError('DRIVER_NOT_INSTALLED');

    assert.ok(error instanceof PattieError);
    assert.equal(error.code, 'DRIVER_NOT_INSTALLED');
    assert.match(error.message, /pattie-cli install/);
});
