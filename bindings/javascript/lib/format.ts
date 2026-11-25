// Audio formats: plain objects, as voice APIs describe their audio.

import type { Format as NativeFormat } from '../native/index.js';

/** How samples are written: 32-bit float, 16-bit PCM, or 8-bit G.711 μ-law. */
export type Encoding = 'f32' | 'pcm16' | 'ulaw';

/** A rate (8,000 to 96,000), an encoding, and 1 to 8 interleaved channels. */
export interface AudioFormat {
    /** Frames a second: 8,000, 11,025, 16,000, 22,050, 24,000, 32,000, 44,100, 48,000 or 96,000. */
    readonly rate: number;
    /** How each sample is written. */
    readonly encoding: Encoding;
    /** 1 when left out. */
    readonly channels?: number;
}

/** A Device's own format, and what its audio is in when no format is given. */
export const DEVICE_FORMAT: Required<AudioFormat> = Object.freeze({
    rate: 48_000,
    encoding: 'f32',
    channels: 1,
});

/** `format` as the native module takes it (it checks the encoding itself). */
export function nativeFormat(format: AudioFormat | undefined): NativeFormat | undefined {
    if (format === undefined) return undefined;
    const { rate, encoding, channels } = format;
    return channels === undefined ? { rate, encoding } : { rate, encoding, channels };
}
