// Playout: a Microphone as a voice agent's output (barge-in).

import { type AudioData, audioBytes } from './bytes.js';
import type { Encoding } from './format.js';
import { call, cancellable, type Native, type WaitOptions } from './native.js';

/** What `interrupt()` cut. */
export interface Interrupted {
    /** The reply's name as given to `push`; null if it had none, or nothing was pushed. */
    readonly reply: string | null;
    /** Frames of the reply listeners took, at the microphone's rate. */
    readonly heardFrames: number;
    /** Frames dropped from the queue. */
    readonly discardedFrames: number;
    /** `heardFrames` in whole milliseconds: OpenAI Realtime's `audio_end_ms`. */
    readonly heardMs: number;
}

/**
 * A Microphone's output for a voice agent (`Microphone.playout()`): audio
 * pushed at any speed is played at the listeners' pace; `interrupt()` stops
 * it at once.
 */
export class Playout implements Disposable {
    readonly #native: Native.Playout;
    readonly #encoding: Encoding;

    /** @internal Made by `Microphone.playout()`. */
    constructor(playout: Native.Playout, encoding: Encoding) {
        this.#native = playout;
        this.#encoding = encoding;
    }

    /**
     * Queues audio (in the playout's format) after what's queued, and returns at
     * once: how many of the microphone's frames were queued. `reply` names the
     * reply it belongs to (OpenAI's item id).
     *
     * @throws `PattieError` `CLOSED` once the playout or its microphone is closed.
     * @throws `TypeError` for a typed array that doesn't match the playout's encoding.
     */
    push(data: AudioData, options: { reply?: string } = {}): number {
        const bytes = audioBytes(data, this.#encoding);
        return call(() => this.#native.push(bytes, options.reply));
    }

    /**
     * Stops the current reply: what's still queued is dropped. What already
     * left for the output (up to 50 ms) still plays.
     *
     * @example
     * ```ts
     * const cut = voice.interrupt();
     * // Tell the model how much of its reply was heard:
     * send({ type: 'conversation.item.truncate', item_id: cut.reply, audio_end_ms: cut.heardMs });
     * ```
     */
    interrupt(): Interrupted {
        const cut = call(() => this.#native.interrupt());
        return Object.freeze({ ...cut, reply: cut.reply ?? null });
    }

    /** Resolves with true once everything pushed has been heard; false if closed first. */
    async waitIdle(options: WaitOptions = {}): Promise<boolean> {
        const { signal } = options;
        const idle = await cancellable(signal, (cancel) => this.#native.waitIdle(cancel));
        signal?.throwIfAborted();
        return idle;
    }

    /** Frames waiting to be played. */
    get queuedFrames(): number {
        return call(() => this.#native.queuedFrames);
    }

    /** Stops playing and gives the microphone's writer back. Closing twice is fine. */
    close(): void {
        this.#native.close();
    }

    /** As `close()`: what leaving a `using` block calls. */
    [Symbol.dispose](): void {
        this.close();
    }
}
