// The system's side in the tests: what Windows' audio engine or PipeWire would do.

import type { FakeBackend } from '@pattie/pattie/_testing';

/** Samples `0, 1/n, 2/n, ...`: audio whose order a test can check. */
export function ramp(frames: number): Float32Array {
    return Float32Array.from({ length: frames }, (_, i) => i / frames);
}

/**
 * The system taking 480 frames of a Microphone's audio every 5 ms, from a timer
 * on the event loop: it gets nothing while the loop is blocked. Stops when
 * disposed (`using`), so a failing test doesn't leave it running.
 *
 * A real system takes 480 frames whatever is there, so a pull before the app's
 * audio arrives is silence (Gap). With `buffered`, it takes only what's there
 * (as the SDK's contract tests do): what's heard is then exactly what was
 * written, in order, however late the write starts.
 */
export function pullInBackground(backend: FakeBackend, deviceId: number, buffered?: () => number) {
    const heard: number[] = [];
    const timer = setInterval(() => {
        const count = buffered === undefined ? 480 : Math.min(480, buffered());
        if (count === 0) return;
        const taken = backend.systemRead(deviceId, count);
        if (taken !== null) heard.push(...taken);
    }, 5);
    return {
        heard,
        [Symbol.dispose]: () => clearInterval(timer),
    };
}

/** The next PATTIE_AUDIO_LOST warning whose message mentions `call`. */
export function nextWarning(call: string): Promise<Error & { code?: string }> {
    return new Promise((resolve) => {
        const listener = (warning: Error & { code?: string }) => {
            if (warning.code === 'PATTIE_AUDIO_LOST' && warning.message.includes(call)) {
                process.off('warning', listener);
                resolve(warning);
            }
        };
        process.on('warning', listener);
    });
}
