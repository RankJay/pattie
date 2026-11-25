// Formats, the Device format, and stable IDs: the values every language shares.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { DEVICE_FORMAT, Microphone, PattieError, stableId } from '@pattie/pattie';
import { BACKEND, FakeBackend } from '#testing';

test('stableId matches the ID Windows and PipeWire give the Device', () => {
    // The same values as the SDK's and the Python binding's stable_id.
    assert.equal(stableId('microphone', 'Agent Mic'), 'M41B28DFC5B21A0BE');
    assert.equal(
        stableId('speaker', 'Agent Speaker', { rate: 96_000, channels: 2 }),
        'S68EF77442E193DC5',
    );
});

test("DEVICE_FORMAT is the Device's own: 48 kHz mono float, frozen", () => {
    assert.deepEqual(DEVICE_FORMAT, { rate: 48_000, encoding: 'f32', channels: 1 });
    assert.ok(Object.isFrozen(DEVICE_FORMAT));
});

test("a format the converter can't take is UNSUPPORTED_FORMAT when the Microphone is made", () => {
    const backend = new FakeBackend();
    assert.throws(
        () =>
            new Microphone('Agent Mic', {
                format: { rate: 12_345, encoding: 'pcm16' },
                [BACKEND]: backend,
            }),
        (error: unknown) => error instanceof PattieError && error.code === 'UNSUPPORTED_FORMAT',
    );
});

test("an encoding that isn't f32, pcm16 or ulaw is a TypeError", () => {
    const backend = new FakeBackend();
    assert.throws(
        () =>
            new Microphone('Agent Mic', {
                // @ts-expect-error: not an Encoding
                format: { rate: 24_000, encoding: 's24' },
                [BACKEND]: backend,
            }),
        TypeError,
    );
});
