// Creating and closing Devices, and the errors that go with them.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { Microphone, PattieError } from '@pattie/pattie';
import { FakeBackend } from '@pattie/pattie/_testing';

test('a second microphone with the same name is DUPLICATE_NAME, and says the fix', () => {
    const backend = new FakeBackend();
    using _mic = new Microphone('Agent Mic', { _backend: backend });

    assert.throws(
        () => new Microphone('Agent Mic', { _backend: backend }),
        (error: unknown) =>
            error instanceof PattieError &&
            error.code === 'DUPLICATE_NAME' &&
            error.message.includes('close the other one first'),
    );
});
