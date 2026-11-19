// Audio as bytes: what the native module takes, from whatever the app holds,
// and what it gives back, as the array that matches the encoding.

import type { Encoding } from './format.js';

/** Audio in a format's encoding: a typed array, a Buffer, or raw bytes. */
export type AudioData = ArrayBufferView | ArrayBuffer;

/** The array a read returns for each encoding. */
export type AudioArray<E extends Encoding> = E extends 'f32'
    ? Float32Array
    : E extends 'pcm16'
      ? Int16Array
      : Uint8Array;

/**
 * The bytes of `data`, within its `byteOffset` and `byteLength` (a Buffer from
 * Node's pool shares its memory with others). Not copied here: the native
 * module copies once. A `Float32Array` must be `f32` audio and an `Int16Array`
 * `pcm16`; bytes (`Uint8Array`, `Buffer`, `DataView`, `ArrayBuffer`) may be any.
 */
export function audioBytes(data: AudioData, encoding: Encoding): Uint8Array {
    if (data instanceof ArrayBuffer) return new Uint8Array(data);
    if (!ArrayBuffer.isView(data)) {
        throw new TypeError('audio must be a typed array, a Buffer or an ArrayBuffer');
    }
    const expected =
        data instanceof Float32Array ? 'f32' : data instanceof Int16Array ? 'pcm16' : undefined;
    const bytes = data instanceof Uint8Array || data instanceof DataView;
    if (!bytes && expected === undefined) {
        throw new TypeError(
            `${data.constructor.name} isn't audio: use a Float32Array (f32), an Int16Array (pcm16) or bytes`,
        );
    }
    if (expected !== undefined && expected !== encoding) {
        throw new TypeError(
            `a ${data.constructor.name} holds ${expected} audio, but this format is ${encoding}`,
        );
    }
    return new Uint8Array(data.buffer, data.byteOffset, data.byteLength);
}

/** Bytes the native module returned, as the array that matches `encoding`. */
export function audioArray<E extends Encoding>(bytes: Uint8Array, encoding: E): AudioArray<E> {
    // The native module's arrays start at offset 0, so they're aligned.
    const { buffer, byteOffset, byteLength } = bytes;
    const array =
        encoding === 'f32'
            ? new Float32Array(buffer, byteOffset, byteLength / 4)
            : encoding === 'pcm16'
              ? new Int16Array(buffer, byteOffset, byteLength / 2)
              : bytes;
    return array as AudioArray<E>;
}
