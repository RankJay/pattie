// Microphone: a virtual microphone other apps record from.

import { type AudioData, audioBytes } from './bytes.js';
import { type AudioFormat, DEVICE_FORMAT, nativeFormat } from './format.js';
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
import { LiveWriter } from './live-writer.js';
import { Playout } from './playout.js';
import type { Stats } from './stats.js';

/** Emits a lost-audio warning the native module returned, if any. */
function warned(written: Native.Written): number {
    if (written.warning != null) {
        process.emitWarning(written.warning, { code: 'PATTIE_AUDIO_LOST' });
    }
    return written.frames;
}

/** How a Microphone is made. */
export interface MicrophoneOptions {
    /** The rate it runs at: 44,100, 48,000 (the default) or 96,000. */
    rate?: number;
    /** 1 (the default) to 8. */
    channels?: number;
    /** What `write` takes: the microphone's own (32-bit float) when left out. */
    format?: AudioFormat;
    /**
     * Lost audio throws `AUDIO_LOST` instead of a `PATTIE_AUDIO_LOST` warning
     * (`process.emitWarning`, at most once a second).
     */
    strict?: boolean;
    /** For tests: a FakeBackend from the package's own `#testing` import. Leave it out. */
    [BACKEND]?: unknown;
}

/**
 * A virtual microphone: what the app writes is what other apps record.
 *
 * @example
 * ```ts
 * using mic = new Microphone('Agent Mic', { format: { rate: 24_000, encoding: 'pcm16' } });
 * await mic.write(pcm); // an Int16Array: resolves as listeners take it
 * await mic.waitForPlayout();
 * ```
 */
export class Microphone implements Disposable, AsyncDisposable {
    readonly #native: Native.Microphone;
    readonly #format: Required<AudioFormat>;
    readonly #rate: number;
    readonly #channels: number;
    /** Calls in progress, which `Symbol.asyncDispose` lets settle. */
    readonly #pending = new Set<Promise<unknown>>();

    /**
     * Creates the microphone; other apps list it by `name` until it's closed.
     *
     * @throws `PattieError` `DRIVER_NOT_INSTALLED` (Windows) or
     * `AUDIO_SERVER_UNAVAILABLE` (Linux); `DUPLICATE_NAME` if a microphone has
     * the name; `TOO_MANY_DEVICES`; `BAD_NAME`; `UNSUPPORTED_FORMAT` for a rate
     * or channel count it can't run at.
     * @throws `TypeError` for a `format` with an unknown encoding.
     */
    constructor(name: string, options: MicrophoneOptions = {}) {
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
            () =>
                new native.Microphone(
                    name,
                    rate,
                    channels,
                    nativeFormat(options.format),
                    options.strict ?? false,
                    backend,
                ),
        );
    }

    /** The driver's ID for this Device. */
    get id(): number {
        return this.#native.id;
    }

    /** What `write` takes. */
    get format(): Required<AudioFormat> {
        return this.#format;
    }

    /** The rate the microphone runs at. */
    get rate(): number {
        return this.#rate;
    }

    /** The microphone's channels. */
    get channels(): number {
        return this.#channels;
    }

    /** Whether `close()` was called. */
    get closed(): boolean {
        return this.#native.closed;
    }

    /** Frames listeners have taken so far. Never waits for a write in progress. */
    get playedFrames(): number {
        return call(() => this.#native.playedFrames);
    }

    /** `playedFrames` in whole milliseconds, as OpenAI Realtime's `audio_end_ms` takes it. */
    get playedMs(): number {
        return call(() => this.#native.playedMs);
    }

    /** Tracks a call in progress for `Symbol.asyncDispose`. */
    #track<T>(promise: Promise<T>): Promise<T> {
        this.#pending.add(promise);
        const forget = () => this.#pending.delete(promise);
        promise.then(forget, forget);
        return promise;
    }

    /**
     * Writes `data` (in this microphone's format) at the pace listeners take it,
     * so nothing is dropped. Resolves with how many of the microphone's frames
     * went in: all of them, unless it was closed meanwhile.
     *
     * @throws `TypeError` for a typed array that doesn't match the format's encoding.
     * @throws `PattieError` `CLOSED`; `ALREADY_TAKEN` while a playout or a
     * live writer has the microphone; `AUDIO_LOST` with `strict`.
     * @throws The signal's reason (an `AbortError` or `TimeoutError`
     * `DOMException`) if `options.signal` aborts first.
     *
     * @example
     * ```ts
     * await mic.write(chunk, { signal: AbortSignal.timeout(5_000) });
     * ```
     */
    write(data: AudioData, options: WaitOptions = {}): Promise<number> {
        return this.#track(this.#write(data, options));
    }

    async #write(data: AudioData, options: WaitOptions): Promise<number> {
        const bytes = audioBytes(data, this.#format.encoding);
        const { signal } = options;
        const frames = warned(
            await cancellable(signal, (cancel) => this.#native.write(bytes, cancel)),
        );
        signal?.throwIfAborted();
        return frames;
    }

    /**
     * Writes what fits right now without waiting; the rest is dropped and counted
     * as Overflow (and warned about, or thrown with `strict`). Returns how many of
     * the microphone's frames went in.
     *
     * @throws `PattieError` `CLOSED`; `ALREADY_TAKEN`; `AUDIO_LOST` with `strict`.
     */
    writeNowait(data: AudioData): number {
        const bytes = audioBytes(data, this.#format.encoding);
        return warned(call(() => this.#native.writeNowait(bytes)));
    }

    /**
     * Resolves with true once listeners have taken everything written so far,
     * false if the microphone was closed first.
     *
     * @throws `PattieError` `CLOSED`; `ALREADY_TAKEN`.
     * @throws The signal's reason if `options.signal` aborts first.
     */
    waitForPlayout(options: WaitOptions = {}): Promise<boolean> {
        return this.#track(this.#waitForPlayout(options));
    }

    async #waitForPlayout(options: WaitOptions): Promise<boolean> {
        const { signal } = options;
        const done = await cancellable(signal, (cancel) => this.#native.waitForPlayout(cancel));
        signal?.throwIfAborted();
        return done;
    }

    /**
     * The microphone as a voice agent's output: push audio as fast as a model
     * sends it, interrupt it the moment someone talks over it. It takes the
     * microphone's one writer, so it can't be mixed with `write()`; closing the
     * microphone closes it too. `format` is what it takes: the microphone's own
     * `format` when left out.
     *
     * @throws `PattieError` `ALREADY_TAKEN` while `write()` or a live writer
     * has the microphone; `CLOSED`; `UNSUPPORTED_FORMAT`.
     *
     * @example
     * ```ts
     * using voice = mic.playout({ rate: 24_000, encoding: 'pcm16' });
     * voice.push(Buffer.from(delta, 'base64'), { reply: itemId });
     * const cut = voice.interrupt(); // someone spoke
     * ```
     */
    playout(format: AudioFormat = this.#format): Playout {
        const playout = call(() => this.#native.playout(nativeFormat(format)));
        return new Playout(playout, format.encoding);
    }

    /**
     * The microphone as the input of a live source: audio relayed in real time
     * from something with its own clock (a network call, a capture card), pushed
     * as it arrives in `format` (the microphone's own when left out). It takes
     * the microphone's one writer; closing the microphone closes it too.
     *
     * @throws `PattieError` `ALREADY_TAKEN` while `write()` or a playout has
     * the microphone; `CLOSED`; `UNSUPPORTED_FORMAT`.
     */
    liveWriter(format: AudioFormat = this.#format): LiveWriter {
        const live = call(() => this.#native.liveWriter(nativeFormat(format)));
        return new LiveWriter(live, format.encoding);
    }

    /** Where listeners had got to at the system's latest pass; null before it first ran. */
    get clock(): ClockPoint | null {
        return clockPoint(call(() => this.#native.clock));
    }

    /**
     * When the next frame written will be taken, in `performance.now()`
     * milliseconds: for keeping audio in step with video. Null before the
     * system first ran.
     */
    get nextPlayTime(): number | null {
        const age = call(() => this.#native.nextPlayAgeMs);
        return age == null ? null : timeOf(age);
    }

    /** Gap, Overflow and what's buffered. Never waits for a write in progress. */
    get stats(): Stats {
        return Object.freeze(call(() => this.#native.stats));
    }

    /** Removes the microphone. A write or wait in progress resolves at once. */
    close(): void {
        const warning = this.#native.close();
        if (warning != null) process.emitWarning(warning, { code: 'PATTIE_AUDIO_LOST' });
    }

    /** As `close()`: what leaving a `using` block calls. */
    [Symbol.dispose](): void {
        this.close();
    }

    /** Closes the microphone, then lets the calls in progress settle. */
    async [Symbol.asyncDispose](): Promise<void> {
        this.close();
        await Promise.allSettled(this.#pending);
    }
}
