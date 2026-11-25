// The private native module, and the error protocol between it and this layer.

import { addAbortListener } from 'node:events';
import { createRequire } from 'node:module';

import type * as Native from '../native/index.js';

export type { Native };

/**
 * Which error a {@link PattieError} is. The same errors as the Python
 * binding's exceptions (ADR 0019).
 */
export type PattieErrorCode =
    /** Windows: the Pattie driver isn't installed. Install it with `pattie-cli install`. */
    | 'DRIVER_NOT_INSTALLED'
    /**
     * Linux: PipeWire isn't running, PulseAudio is the sound server, or a
     * Flatpak sandbox can't reach PipeWire.
     */
    | 'AUDIO_SERVER_UNAVAILABLE'
    /** This user may not open the driver. */
    | 'ACCESS_DENIED'
    /** No backend for this operating system yet. */
    | 'NO_BACKEND'
    /** The installed driver doesn't match this package's version. */
    | 'VERSION_MISMATCH'
    /** A rate or channel count a Device or the converter can't take. */
    | 'UNSUPPORTED_FORMAT'
    /** A buffer size that isn't a power of two from 256 to 65,536 frames. */
    | 'BAD_BUFFER_SIZE'
    /** The name is empty, longer than 63 characters, or has control characters. */
    | 'BAD_NAME'
    /** A Device of this kind already has the name. */
    | 'DUPLICATE_NAME'
    /** At most 8 Devices at a time. */
    | 'TOO_MANY_DEVICES'
    /** A writer asked of a Speaker, or a reader of a Microphone. */
    | 'WRONG_KIND'
    /** A Microphone has one writer: `write()`, a playout or a live writer. */
    | 'ALREADY_TAKEN'
    /** Windows refused to create the endpoint, or was busy (try again). */
    | 'SYSTEM_REFUSED'
    /** The driver rejected a request this package built: a bug to report. */
    | 'PROTOCOL_ERROR'
    /** An operating-system error; the message has its number. */
    | 'OS_ERROR'
    /** The driver was updated while the Device was live: create it again. */
    | 'DRIVER_REMOVED'
    /** Audio was dropped or ran out, with `strict`. */
    | 'AUDIO_LOST'
    /** The Device was closed. */
    | 'CLOSED'
    /** An SDK error this package has no code for yet; the message says which. */
    | 'ERROR'
    /**
     * Importing the package on a platform it has no binary for. It runs on
     * Windows x64, and Linux x64 and arm64 with glibc 2.28 or newer.
     */
    | 'UNSUPPORTED_PLATFORM';

/**
 * Every error Pattie throws or rejects with, apart from a `TypeError` for an
 * argument of the wrong type and the signal's reason for an aborted wait.
 * Its message says the fix.
 *
 * @example
 * ```ts
 * try {
 *     mic = new Microphone('Agent Mic');
 * } catch (error) {
 *     if (error instanceof PattieError && error.code === 'DRIVER_NOT_INSTALLED') {
 *         console.error(error.message); // names the command that installs it
 *     } else throw error;
 * }
 * ```
 */
export class PattieError extends Error {
    /** Which error it is, for code to branch on; the message is for people. */
    readonly code: PattieErrorCode;

    constructor(code: PattieErrorCode, message: string, options?: ErrorOptions) {
        super(message, options);
        this.name = 'PattieError';
        this.code = code;
    }
}

/** The platforms the package carries a binary for, as an error message names them. */
const SUPPORTED = 'Windows x64, and Linux x64 and arm64 with glibc 2.28 or newer';

/** This platform as a binary's name has it (`win32-x64-msvc`, `linux-arm64-gnu`). */
function platformName(): string {
    const abi = process.platform === 'win32' ? '-msvc' : process.platform === 'linux' ? '-gnu' : '';
    return `${process.platform}-${process.arch}${abi}`;
}

/** Linux without glibc (Alpine's musl, say), which the binaries can't run on. */
function isMusl(): boolean {
    if (process.platform !== 'linux') return false;
    const report = process.report?.getReport() as { header?: { glibcVersionRuntime?: string } };
    return report?.header?.glibcVersionRuntime === undefined;
}

/**
 * This platform's binary, of the ones the package carries. It replaces the
 * napi-rs loader, which looks for per-platform npm packages this package
 * doesn't have, and blames npm when there's none.
 */
function load(): typeof Native {
    const require = createRequire(import.meta.url);
    const file = `../native/pattie.${platformName()}.node`;
    try {
        return require(file) as typeof Native;
    } catch (error) {
        const here = `${platformName()}${isMusl() ? ' (musl)' : ''}`;
        throw new PattieError(
            'UNSUPPORTED_PLATFORM',
            `Pattie has no binary for ${here}: it runs on ${SUPPORTED}`,
            { cause: error },
        );
    }
}

/** The native module: this platform's binary. */
export const native: typeof Native = load();

const CODED = /^([A-Z][A-Z_]*): ([\s\S]*)$/;

/**
 * A native `<CODE>: <message>` error as JavaScript's: PattieError or
 * TypeError, with the native error as its `cause`.
 */
export function translated(error: unknown): unknown {
    if (!(error instanceof Error)) return error;
    const match = CODED.exec(error.message);
    if (match === null) return error;
    const [, code = '', message = ''] = match;
    const options = { cause: error };
    return code === 'TYPE_ERROR'
        ? new TypeError(message, options)
        : new PattieError(code as PattieErrorCode, message, options);
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
    run: (cancel: Native.Cancel | undefined) => Promise<T>,
): Promise<T> {
    if (signal === undefined) return settled(() => run(undefined));
    signal.throwIfAborted();
    const cancel = new native.Cancel();
    // addAbortListener: still called if other code stops the event's propagation.
    using _listening = addAbortListener(signal, () => cancel.cancel());
    return await settled(() => run(cancel));
}

/** How `#testing`'s FakeBackend hands its native Backend to a Device. */
export const BACKEND: unique symbol = Symbol('pattie.backend');

/** The native Backend a hidden `[BACKEND]` option names, if any. */
export function backendOf(backend: unknown): Native.Backend | undefined {
    if (backend === undefined) return undefined;
    if (typeof backend === 'object' && backend !== null && BACKEND in backend) {
        return (backend as { [BACKEND](): Native.Backend })[BACKEND]();
    }
    throw new TypeError("[BACKEND] must be a FakeBackend from the tests' #testing import");
}
