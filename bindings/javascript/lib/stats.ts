// Stats: a Device's counters, in frames.

/** What a Device has lost and holds, in frames. */
export interface Stats {
    /** Silence played because the app wrote too late (Microphone), or audio dropped because it read too late (Speaker). */
    readonly gapFrames: number;
    /** Audio dropped because the Ring was full. */
    readonly overflowFrames: number;
    /** Frames in the Ring now. */
    readonly bufferedFrames: number;
}
