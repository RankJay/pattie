// Speaker: a virtual speaker other apps play to.

import { type AudioArray, audioArray } from './bytes.js';
import { type AudioFormat, DEVICE_FORMAT, type Encoding, nativeFormat } from './format.js';
import { backendOf, call, cancellable, native, type WaitOptions } from './native.js';
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
    /** For tests: a FakeBackend from `pattie/_testing`. Leave it out. */
    _backend?: unknown;
}

/**
 * A virtual speaker: what other apps play to it is what the app reads, as a
 * `Float32Array`, an `Int16Array` (pcm16) or a `Uint8Array` (μ-law).
 */
export class Speaker<E extends Encoding = 'f32'>
    implements Disposable, AsyncDisposable, AsyncIterable<AudioArray<E>>
{
    readonly #native: native.Speaker;
    readonly #format: Required<AudioFormat>;
    readonly #rate: number;
    readonly #channels: number;
    readonly #pending = new Set<Promise<unknown>>();

    /** Creates the speaker; other apps list it by `name` until it's closed. */
    constructor(name: string, options: SpeakerOptions<E> = {}) {
        const backend = backendOf(options._backend);
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

    /** What's played, a read at a time, until the speaker is closed. */
    async *[Symbol.asyncIterator](): AsyncIterator<AudioArray<E>> {
        while (!this.closed) {
            const chunk = await this.read();
            if (chunk.length === 0) return;
            yield chunk;
        }
    }

    /** Removes the speaker. A read in progress resolves at once, with nothing. */
    close(): void {
        this.#native.close();
    }

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
