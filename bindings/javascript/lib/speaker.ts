// Speaker: a virtual speaker other apps play to.

import { type AudioArray, audioArray } from './bytes.js';
import { type AudioFormat, DEVICE_FORMAT, type Encoding, nativeFormat } from './format.js';
import {
    BACKEND,
    backendOf,
    call,
    cancellable,
    native,
    type Native,
    type WaitOptions,
} from './native.js';
import { type ClockPoint, clockPoint, timeOf } from './clock.js';
import type { Stats } from './stats.js';

/** How a Speaker is made. */
export interface SpeakerOptions<E extends Encoding = 'f32'> {
    /** The rate it runs at: 44,100, 48,000 (the default) or 96,000. */
    rate?: number;
    /** 1 (the default) to 8. */
    channels?: number;
    /** What `read` returns: the speaker's own (32-bit float) when left out. */
    format?: AudioFormat & { encoding: E };
    /** For tests: a FakeBackend from the package's own `#testing` import. Leave it out. */
    [BACKEND]?: unknown;
}

/**
 * A virtual speaker: what other apps play to it is what the app reads, as a
 * `Float32Array`, an `Int16Array` (pcm16) or a `Uint8Array` (μ-law).
 *
 * @example
 * ```ts
 * using speaker = new Speaker('Agent Speaker', { format: { rate: 16_000, encoding: 'pcm16' } });
 * for await (const chunk of speaker) transcriber.feed(chunk); // an Int16Array
 * ```
 */
export class Speaker<E extends Encoding = 'f32'>
    implements Disposable, AsyncDisposable, AsyncIterable<AudioArray<E>>
{
    readonly #native: Native.Speaker;
    readonly #format: Required<AudioFormat>;
    readonly #rate: number;
    readonly #channels: number;
    readonly #pending = new Set<Promise<unknown>>();

    /**
     * Creates the speaker; other apps list it by `name` until it's closed.
     *
     * @throws `PattieError` `DRIVER_NOT_INSTALLED` (Windows) or
     * `AUDIO_SERVER_UNAVAILABLE` (Linux); `DUPLICATE_NAME` if a speaker has the
     * name; `TOO_MANY_DEVICES`; `BAD_NAME`; `UNSUPPORTED_FORMAT`.
     * @throws `TypeError` for a `format` with an unknown encoding.
     */
    constructor(name: string, options: SpeakerOptions<E> = {}) {
        const backend = backendOf(options[BACKEND]);
        const rate = options.rate ?? DEVICE_FORMAT.rate;
        const channels = options.channels ?? DEVICE_FORMAT.channels;
        this.#rate = rate;
        this.#channels = channels;
        this.#format = Object.freeze({
            channels: 1,
            ...(options.format ?? { ...DEVICE_FORMAT, rate, channels }),
        });
        this.#native = call(
            () => new native.Speaker(name, rate, channels, nativeFormat(options.format), backend),
        );
    }

    /** The driver's ID for this Device. */
    get id(): number {
        return this.#native.id;
    }

    /** What `read` returns. */
    get format(): Required<AudioFormat> {
        return this.#format;
    }

    /** The rate the speaker runs at. */
    get rate(): number {
        return this.#rate;
    }

    /** The speaker's channels. */
    get channels(): number {
        return this.#channels;
    }

    /** Whether `close()` was called. */
    get closed(): boolean {
        return this.#native.closed;
    }

    /** Gap, Overflow and what's buffered. Never waits for a read in progress. */
    get stats(): Stats {
        return Object.freeze(call(() => this.#native.stats));
    }

    /**
     * Resolves with what other apps played since the last read, up to 100 ms of
     * it, waiting until there's some. Empty once the speaker is closed.
     *
     * @throws The signal's reason (an `AbortError` or `TimeoutError`
     * `DOMException`) if `options.signal` aborts first.
     */
    read(options: WaitOptions = {}): Promise<AudioArray<E>> {
        return this.#track(this.#read(options));
    }

    async #read(options: WaitOptions): Promise<AudioArray<E>> {
        const { signal } = options;
        const bytes = await cancellable(signal, (cancel) => this.#native.read(cancel));
        signal?.throwIfAborted();
        return audioArray(bytes, this.#format.encoding as E);
    }

    /**
     * As `read`, with when the first frame returned was played, in
     * `performance.now()` milliseconds (null with nothing): the reference an echo
     * canceller aligns the microphone with.
     */
    readTimed(options: WaitOptions = {}): Promise<{ data: AudioArray<E>; firstAt: number | null }> {
        return this.#track(this.#readTimed(options));
    }

    async #readTimed(
        options: WaitOptions,
    ): Promise<{ data: AudioArray<E>; firstAt: number | null }> {
        const { signal } = options;
        const timed = await cancellable(signal, (cancel) => this.#native.readTimed(cancel));
        const firstAt = timed.firstAgeMs == null ? null : timeOf(timed.firstAgeMs);
        signal?.throwIfAborted();
        return { data: audioArray(timed.data, this.#format.encoding as E), firstAt };
    }

    /** Where the system had got to at its latest pass; null before it first ran. */
    get clock(): ClockPoint | null {
        return clockPoint(call(() => this.#native.clock));
    }

    /**
     * What's played, a read at a time, until the speaker is closed or
     * `options.signal` aborts. Leaving the loop early (`break`) doesn't close
     * the speaker.
     *
     * @throws The signal's reason if `options.signal` aborts.
     *
     * @example
     * ```ts
     * for await (const chunk of speaker.values({ signal: AbortSignal.timeout(60_000) })) {
     *     transcriber.feed(chunk);
     * }
     * ```
     */
    async *values(options: WaitOptions = {}): AsyncGenerator<AudioArray<E>, void, undefined> {
        while (!this.closed) {
            const chunk = await this.read(options);
            if (chunk.length === 0) return;
            yield chunk;
        }
    }

    /** As `values()`: `for await (const chunk of speaker)`. */
    [Symbol.asyncIterator](): AsyncIterator<AudioArray<E>> {
        return this.values();
    }

    /** Removes the speaker. A read in progress resolves at once, with nothing. */
    close(): void {
        this.#native.close();
    }

    /** As `close()`: what leaving a `using` block calls. */
    [Symbol.dispose](): void {
        this.close();
    }

    /** Closes the speaker, then lets the calls in progress settle. */
    async [Symbol.asyncDispose](): Promise<void> {
        this.close();
        await Promise.allSettled(this.#pending);
    }

    #track<T>(promise: Promise<T>): Promise<T> {
        this.#pending.add(promise);
        const forget = () => this.#pending.delete(promise);
        promise.then(forget, forget);
        return promise;
    }
}
