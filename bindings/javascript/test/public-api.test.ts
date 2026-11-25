// What the package shows its users, and what it keeps to itself.

import assert from 'node:assert/strict';
import { test } from 'node:test';

import * as pattie from '@pattie/pattie';
import type { MicrophoneOptions, SpeakerOptions } from '@pattie/pattie';

test('a Playout and a LiveWriter come only from a Microphone', () => {
    // Their types are exported; their classes (and constructors) aren't.
    assert.equal('Playout' in pattie, false);
    assert.equal('LiveWriter' in pattie, false);
});

/** Never called: the compiler checks these lines, which must not compile. */
export function notInTheTypes(): void {
    // @ts-expect-error The tests' backend option isn't one users see.
    const mic: MicrophoneOptions = { _backend: undefined };
    // @ts-expect-error Nor on a Speaker.
    const speaker: SpeakerOptions = { _backend: undefined };
    void [mic, speaker];
}
