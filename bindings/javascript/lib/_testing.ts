// For tests only, in dev builds: the SDK's Fake backend, which plays the
// system's side of each Device. Release builds don't have this module.

import {
    BACKEND,
    call,
    native,
    type Native,
    type PattieError,
    type PattieErrorCode,
    translated,
} from './native.js';

/** The option key that hands a Device a FakeBackend: `{ [BACKEND]: backend }`. */
export { BACKEND };

/** An error only a real driver gives (`DRIVER_NOT_INSTALLED`, `DRIVER_REMOVED`, ...), as it reads. */
export function sdkError(code: PattieErrorCode): PattieError {
    return translated(new Error(call(() => native.sdkErrorMessage(code)))) as PattieError;
}

/** The system's side of every Device made with `{ [BACKEND]: this }`. */
export class FakeBackend {
    readonly #native: Native.FakeBackend;

    constructor() {
        const fake = call(() => new native.FakeBackend());
        this.#native = fake;
        // How a Device takes its native Backend (backendOf). Defined here, not
        // as a method: isolatedDeclarations can't type a member named by an
        // imported symbol.
        Object.defineProperty(this, BACKEND, { value: (): Native.Backend => fake.backend() });
    }

    /** The system takes `count` frames of Microphone `id`'s audio; null if no such Device. */
    systemRead(id: number, count: number): Float32Array | null {
        return this.#native.systemRead(id, count);
    }

    /** The system plays `samples` (the Device's own format) to Speaker `id`; false if no such Device. */
    systemWrite(id: number, samples: Float32Array): boolean {
        return this.#native.systemWrite(id, samples);
    }
}
