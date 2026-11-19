// On a real PipeWire (Linux), with PATTIE_PIPEWIRE=1: the binding over the
// SDK's own Linux backend, not the Fake one. CI runs it on a headless PipeWire.

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { test } from 'node:test';

import { Microphone, stableId } from 'pattie';

const skip =
    process.platform !== 'linux' || process.env.PATTIE_PIPEWIRE !== '1'
        ? 'set PATTIE_PIPEWIRE=1 on a Linux system running PipeWire'
        : false;

/** The PipeWire node names now listed. */
function nodes(): string {
    return execFileSync('pw-cli', ['ls', 'Node'], { encoding: 'utf8' });
}

test(
    'a Microphone is a PipeWire node while it lives, and plays what is written',
    { skip },
    async () => {
        const name = `Pattie JS ${process.pid}`;
        {
            using mic = new Microphone(name);
            assert.match(nodes(), new RegExp(`pattie.${stableId('microphone', name)}`));

            // PipeWire's graph pulls a quantum at a time; a paced write finishes
            // only if it does.
            const written = await mic.write(new Float32Array(9_600), {
                signal: AbortSignal.timeout(5_000),
            });
            assert.equal(written, 9_600);
        }
        assert.doesNotMatch(nodes(), new RegExp(`pattie.${stableId('microphone', name)}`));
    },
);
