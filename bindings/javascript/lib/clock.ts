// Times on performance.now()'s timeline, from the ages the native module gives.

import type { ClockAge } from '../native/index.js';

/** Where the system had got to at its latest pass. */
export interface ClockPoint {
    /** Frames the system had taken (Microphone) or played (Speaker) by then. */
    readonly frames: number;
    /** When that pass was, in `performance.now()` milliseconds. */
    readonly at: number;
}

/** A time `ageMs` ago (to come, when negative), on `performance.now()`'s timeline. */
export function timeOf(ageMs: number): number {
    return performance.now() - ageMs;
}

/** A native clock as a ClockPoint, or null before the system first ran. */
export function clockPoint(clock: ClockAge | null | undefined): ClockPoint | null {
    return clock == null ? null : Object.freeze({ frames: clock.frames, at: timeOf(clock.ageMs) });
}
