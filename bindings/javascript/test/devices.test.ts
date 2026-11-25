// Creating and closing Devices, and the errors that go with them.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { Microphone, PattieError, type PattieErrorCode } from '@pattie/pattie';
import { BACKEND, FakeBackend } from '#testing';

test('a second microphone with the same name is DUPLICATE_NAME, and says the fix', () => {
    const backend = new FakeBackend();
    using _mic = new Microphone('Agent Mic', { [BACKEND]: backend });

    assert.throws(
        () => new Microphone('Agent Mic', { [BACKEND]: backend }),
        (error: unknown) =>
            error instanceof PattieError &&
            error.code === 'DUPLICATE_NAME' &&
            error.message.includes('close the other one first'),
    );
});

/** What an app does with a code: a switch the compiler checks covers every one. */
function fixFor(code: PattieErrorCode): string {
    switch (code) {
        case 'DRIVER_NOT_INSTALLED':
        case 'DRIVER_REMOVED':
        case 'VERSION_MISMATCH':
            return 'reinstall';
        case 'DUPLICATE_NAME':
        case 'TOO_MANY_DEVICES':
            return 'close another';
        case 'AUDIO_SERVER_UNAVAILABLE':
        case 'ACCESS_DENIED':
        case 'NO_BACKEND':
        case 'UNSUPPORTED_FORMAT':
        case 'BAD_BUFFER_SIZE':
        case 'BAD_NAME':
        case 'WRONG_KIND':
        case 'ALREADY_TAKEN':
        case 'SYSTEM_REFUSED':
        case 'PROTOCOL_ERROR':
        case 'OS_ERROR':
        case 'AUDIO_LOST':
        case 'CLOSED':
        case 'ERROR':
        case 'UNSUPPORTED_PLATFORM':
            return 'other';
        default: {
            const unknown: never = code;
            return unknown;
        }
    }
}

test('an error code is one of PattieErrorCode, and the native error is its cause', () => {
    const backend = new FakeBackend();
    using _mic = new Microphone('Agent Mic', { [BACKEND]: backend });

    let caught: unknown;
    try {
        new Microphone('Agent Mic', { [BACKEND]: backend });
    } catch (error) {
        caught = error;
    }

    assert.ok(caught instanceof PattieError);
    assert.equal(fixFor(caught.code), 'close another');
    assert.ok(caught.cause instanceof Error);
    assert.match(caught.cause.message, /^DUPLICATE_NAME: /);
});
