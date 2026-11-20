// The package as users get it: a release build, packed as it would publish,
// installed into an empty project, used from outside. Run after `pnpm build`
// (not build:dev): `pnpm test:release`.

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { test } from 'node:test';

const root = resolve(import.meta.dirname, '..', '..');
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
const run = (args: string[], cwd: string) =>
    execFileSync(npm, args, { cwd, encoding: 'utf8', shell: process.platform === 'win32' });

/** The tarball a user installs: `@pattie/pattie`, its binaries inside. */
function pack(into: string): string {
    const binary = readdirSync(join(root, 'native')).find((file) => file.endsWith('.node'));
    assert.ok(binary, 'no release build: run `pnpm build` first');
    const name = run(['pack', '--silent', '--pack-destination', into], root)
        .trim()
        .split(/\r?\n/)
        .at(-1);
    assert.ok(name, 'npm pack printed no file name');
    return join(into, name);
}

const project = mkdtempSync(join(tmpdir(), 'pattie-release-'));
writeFileSync(join(project, 'package.json'), '{ "type": "module", "private": true }');
run(['install', '--silent', '--no-audit', '--no-fund', pack(project)], project);

/** Runs `code` as an ES module in the installed project; its output. */
function inProject(code: string): string {
    return execFileSync(process.execPath, ['--input-type=module', '-e', code], {
        cwd: project,
        encoding: 'utf8',
    }).trim();
}

test('the installed package loads its binary and works', () => {
    assert.equal(
        inProject(
            "import { stableId } from '@pattie/pattie'; console.log(stableId('microphone', 'Agent Mic'))",
        ),
        'M41B28DFC5B21A0BE',
    );
});

test('the installed package has its types', () => {
    const files = run(['ls', '--parseable', '--depth=0'], project);
    assert.match(files, /@pattie[\\/]pattie/);
    assert.equal(
        inProject(
            "import { existsSync } from 'node:fs'; console.log(existsSync('node_modules/@pattie/pattie/dist/index.d.ts'))",
        ),
        'true',
    );
});

test('@pattie/pattie/_testing is not in a release', () => {
    assert.throws(
        () => inProject("await import('@pattie/pattie/_testing')"),
        /ERR_MODULE_NOT_FOUND|Cannot find module/,
    );
});

test('a release build has no Fake backend in its binary', () => {
    // The loader's exports, straight from the binary: FakeBackend is dev-only.
    assert.equal(
        inProject(
            "const native = await import('./node_modules/@pattie/pattie/native/index.js'); console.log('FakeBackend' in native)",
        ),
        'false',
    );
});
