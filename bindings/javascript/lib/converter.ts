// Converter: the SDK's format converter, for audio that isn't a Device's.

import { type AudioArray, type AudioData, audioArray, audioBytes } from './bytes.js';
import { type AudioFormat, type Encoding, nativeFormat } from './format.js';
import { call, native, type Native } from './native.js';

/**
 * Converts audio from one format to another: rate, encoding and channels. It
 * keeps its state between calls, so a stream converts the same in any pieces.
 */
export class Converter<E extends Encoding = Encoding> {
    readonly #native: Native.Converter;
    readonly #source: Encoding;
    readonly #target: E;

    /**
     * @throws `PattieError` `UNSUPPORTED_FORMAT` for a pair it can't join
     * (channels convert only to or from mono).
     *
     * @example
     * ```ts
     * const toModel = new Converter(DEVICE_FORMAT, { rate: 24_000, encoding: 'pcm16' });
     * const pcm = toModel.convert(samples); // an Int16Array
     * ```
     */
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
