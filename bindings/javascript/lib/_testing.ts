// For tests only, in dev builds: the SDK's Fake backend, which plays the
// system's side of each Device. Release builds don't have this module.

import { BACKEND, call, native, type PattieError, translated } from './native.js';

/** An error only a real driver gives (`DRIVER_NOT_INSTALLED`, `DRIVER_REMOVED`, ...), as it reads. */
export function sdkError(code: string): PattieError {
    return translated(new Error(call(() => native.sdkErrorMessage(code)))) as PattieError;
}

/** The system's side of every Device made with `{ _backend: this }`. */
export class FakeBackend {
    readonly #native: native.FakeBackend;

    constructor() {
        this.#native = call(() => new native.FakeBackend());
    }

    /** The system takes `count` frames of Microphone `id`'s audio; null if no such Device. */
    systemRead(id: number, count: number): Float32Array | null {
        return this.#native.systemRead(id, count);
    }

    /** The system plays `samples` (the Device's own format) to Speaker `id`; false if no such Device. */
    systemWrite(id: number, samples: Float32Array): boolean {
        return this.#native.systemWrite(id, samples);
    }

    [BACKEND](): native.Backend {
        return this.#native.backend();
    }
}
