// Converter: the SDK's format converter, for audio that isn't a Device's.

import { type AudioArray, type AudioData, audioArray, audioBytes } from './bytes.js';
import { type AudioFormat, type Encoding, nativeFormat } from './format.js';
import { call, native } from './native.js';

/**
 * Converts audio from one format to another: rate, encoding and channels. It
 * keeps its state between calls, so a stream converts the same in any pieces.
 */
export class Converter<E extends Encoding = Encoding> {
    readonly #native: native.Converter;
    readonly #source: Encoding;
    readonly #target: E;

    constructor(source: AudioFormat, target: AudioFormat & { encoding: E }) {
        this.#native = call(
            () => new native.Converter(nativeFormat(source)!, nativeFormat(target)!),
        );
        this.#source = source.encoding;
        this.#target = target.encoding;
    }

    /** `data` (in the source format) in the target format. */
    convert(data: AudioData): AudioArray<E> {
        const bytes = audioBytes(data, this.#source);
        return audioArray(
            call(() => this.#native.convert(bytes)),
            this.#target,
        );
    }
}
