// LiveWriter: a Microphone's input for a live source with its own clock.

import { type AudioData, audioBytes } from './bytes.js';
import type { Encoding } from './format.js';
import { call, type Native } from './native.js';

/** A LiveWriter's jitter buffer. */
export interface LiveStats {
    /** Frames waiting (about 40 ms when steady). */
    readonly queuedFrames: number;
    /** How far the source is sped up (+) or slowed down (-), in parts per million. */
    readonly correctionPpm: number;
    /** Frames the source didn't deliver in time (played as silence). */
    readonly starvedFrames: number;
    /** Frames dropped because the buffer grew too long. */
    readonly droppedFrames: number;
}

/**
 * A Microphone's input for a live source (`Microphone.liveWriter()`): audio
 * relayed in real time from a network call or a capture card, pushed as it
 * arrives. For a voice agent's replies, use `playout()`.
 */
export class LiveWriter implements Disposable {
    readonly #native: Native.LiveWriter;
    readonly #encoding: Encoding;

    /** @internal Made by `Microphone.liveWriter()`. */
    constructor(live: Native.LiveWriter, encoding: Encoding) {
        this.#native = live;
        this.#encoding = encoding;
    }

    /**
     * Adds audio as it arrives; never waits. Returns how many of the
     * microphone's frames that adds.
     *
     * @throws `PattieError` `CLOSED` once the writer or its microphone is closed.
     * @throws `TypeError` for a typed array that doesn't match the writer's encoding.
     */
    push(data: AudioData): number {
        const bytes = audioBytes(data, this.#encoding);
        return call(() => this.#native.push(bytes));
    }

    /** The jitter buffer's state. Never waits for a push. */
    get stats(): LiveStats {
        return Object.freeze(call(() => this.#native.stats));
    }

    /** Stops it and gives the microphone's writer back. Closing twice is fine. */
    close(): void {
        this.#native.close();
    }

    /** As `close()`: what leaving a `using` block calls. */
    [Symbol.dispose](): void {
        this.close();
    }
}
