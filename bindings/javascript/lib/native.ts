// The private native module, and the error protocol between it and this layer.

import * as native from '../native/index.js';

export { native };

/** What every Pattie error is: one class, with a `code` saying which. */
export class PattieError extends Error {
    readonly code: string;

    constructor(code: string, message: string) {
        super(message);
        this.name = 'PattieError';
        this.code = code;
    }
}

const CODED = /^([A-Z][A-Z_]*): ([\s\S]*)$/;

/** A native `<CODE>: <message>` error as JavaScript's: PattieError or TypeError. */
export function translated(error: unknown): unknown {
    if (!(error instanceof Error)) return error;
    const match = CODED.exec(error.message);
    if (match === null) return error;
    const [, code = '', message = ''] = match;
    return code === 'TYPE_ERROR' ? new TypeError(message) : new PattieError(code, message);
}

/**
 * Runs a native call, translating what it throws. The error's stack starts at
 * the public method that called this, not inside the package.
 */
export function call<T>(run: () => T): T {
    try {
        return run();
    } catch (error) {
        const thrown = translated(error);
        if (thrown instanceof Error) Error.captureStackTrace(thrown, call);
        throw thrown;
    }
}

/** Runs a native call that returns a Promise, translating what either throws. */
export async function settled<T>(run: () => Promise<T>): Promise<T> {
    try {
        return await run();
    } catch (error) {
        throw translated(error);
    }
}

/** The options every call that waits takes. */
export interface WaitOptions {
    /** Ends the wait (within about 100 ms); the call rejects with its reason. */
    signal?: AbortSignal;
}

/**
 * Runs a native call that waits, with a `Cancel` the signal cancels. The caller
 * calls `signal?.throwIfAborted()` after using the result, so what the call did
 * before the abort (a warning) isn't lost.
 */
export async function cancellable<T>(
    signal: AbortSignal | undefined,
    run: (cancel: native.Cancel | undefined) => Promise<T>,
): Promise<T> {
    if (signal === undefined) return settled(() => run(undefined));
    signal.throwIfAborted();
    const cancel = new native.Cancel();
    const abort = () => cancel.cancel();
    signal.addEventListener('abort', abort, { once: true });
    try {
        return await settled(() => run(cancel));
    } finally {
        signal.removeEventListener('abort', abort);
    }
}

/** How `pattie/_testing`'s FakeBackend hands its native Backend to a Device. */
export const BACKEND: unique symbol = Symbol('pattie.backend');

/** The native Backend a hidden `_backend` option names, if any. */
export function backendOf(backend: unknown): native.Backend | undefined {
    if (backend === undefined) return undefined;
    if (typeof backend === 'object' && backend !== null && BACKEND in backend) {
        return (backend as { [BACKEND](): native.Backend })[BACKEND]();
    }
    throw new TypeError('_backend must be a FakeBackend from pattie/_testing');
}
