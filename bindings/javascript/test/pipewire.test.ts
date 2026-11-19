// On a real PipeWire (Linux), with PATTIE_PIPEWIRE=1: the binding over the
// SDK's own Linux backend, not the Fake one. CI runs it on a headless PipeWire.
// It needs pw-record: a headless PipeWire has no devices, so nothing pulls a
// microphone's audio until something records it.

import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { setTimeout as sleep } from 'node:timers/promises';

import { Microphone, stableId } from 'pattie';

const skip =
    process.platform !== 'linux' || process.env.PATTIE_PIPEWIRE !== '1'
        ? 'set PATTIE_PIPEWIRE=1 on a Linux system running PipeWire, with pw-record'
        : false;

/** The PipeWire node names now listed. */
function nodes(): string {
    return execFileSync('pw-cli', ['ls', 'Node'], { encoding: 'utf8' });
}

/** The float samples of pw-record's WAV file (its `data` chunk). */
function wavSamples(path: string): Float32Array {
    const data = readFileSync(path);
    for (let at = 12; at + 8 <= data.length;) {
        const size = data.readUInt32LE(at + 4);
        if (data.toString('latin1', at, at + 4) === 'data') {
            const bytes = data.subarray(at + 8, at + 8 + size - (size % 4));
            return new Float32Array(
                bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.length),
            );
        }
        at += 8 + size + (size & 1);
    }
    return new Float32Array();
}

test(
    'a recording of the Microphone hears what is written, and the node goes at close',
    { skip },
    async () => {
        const name = `Pattie JS ${process.pid}`;
        // node.name is "pattie." and the stable ID (the Linux backend).
        const node = `pattie.${stableId('microphone', name)}`;
        const wav = join(mkdtempSync(join(tmpdir(), 'pattie-pw-')), 'heard.wav');
        {
            using mic = new Microphone(name);
            assert.ok(nodes().includes(node), `${node} isn't listed`);

            const recorder = spawn('pw-record', [
                '--target',
                node,
                '--rate',
                '48000',
                '--channels',
                '1',
                '--format',
                'f32',
                wav,
            ]);
            try {
                await sleep(500); // the recorder links up
                // 200 ms at 0.5: a paced write finishes only if the recording pulls it.
                const written = await mic.write(new Float32Array(9_600).fill(0.5), {
                    signal: AbortSignal.timeout(5_000),
                });
                assert.equal(written, 9_600);
                await mic.waitForPlayout({ signal: AbortSignal.timeout(5_000) });
                await sleep(200); // what was pulled reaches the file
            } finally {
                recorder.kill('SIGTERM');
                await once(recorder, 'exit');
            }
        }
        assert.ok(!nodes().includes(node), `${node} is still listed after close`);

        const heard = wavSamples(wav);
        const written = heard.filter((sample) => Math.abs(sample - 0.5) < 0.01).length;
        assert.ok(
            written >= 9_000,
            `heard ${heard.length} samples, ${written} of them the 0.5 written`,
        );
    },
);
