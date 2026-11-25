/**
 * Virtual microphones and speakers that other apps list as real audio
 * devices: write to a {@link Microphone} and Zoom, Teams or a browser records
 * it; other apps play to a {@link Speaker} and the app reads it.
 *
 * @packageDocumentation
 */

import { call, native } from './native.js';

export type { AudioArray, AudioData } from './bytes.js';
export type { ClockPoint } from './clock.js';
export { Converter } from './converter.js';
export { type AudioFormat, DEVICE_FORMAT, type Encoding } from './format.js';
export type { LiveStats, LiveWriter } from './live-writer.js';
export { Microphone, type MicrophoneOptions } from './microphone.js';
export type { Interrupted, Playout } from './playout.js';
export { Speaker, type SpeakerOptions } from './speaker.js';
export { PattieError, type PattieErrorCode, type WaitOptions } from './native.js';
export type { Stats } from './stats.js';

/** Which kind of Device. */
export type Kind = 'microphone' | 'speaker';

/** The Device's stable ID: what Windows and PipeWire will call it, before it exists. */
export function stableId(
    kind: Kind,
    name: string,
    options: { rate?: number; channels?: number } = {},
): string {
    return call(() => native.stableId(kind, name, options.rate ?? 48_000, options.channels ?? 1));
}
