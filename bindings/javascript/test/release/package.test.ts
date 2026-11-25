// The package as users get it: a release build, packed as it would publish,
// installed into an empty project, used from outside. Run after `pnpm build`
// (not build:dev): `pnpm test:release`.

import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { cpSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
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
const tarball = pack(project);
run(['install', '--silent', '--no-audit', '--no-fund', tarball], project);

/** Each dev tool's CLI script, run with this Node (no shell, so the same on Windows). */
const CLI = {
    publint: 'node_modules/publint/src/cli.js',
    attw: 'node_modules/@arethetypeswrong/cli/dist/index.js',
};

/** Runs one of this package's dev tools; whether it passed, and what it printed. */
function tool(name: keyof typeof CLI, args: string[]): { ok: boolean; output: string } {
    try {
        const output = execFileSync(process.execPath, [join(root, CLI[name]), ...args], {
            encoding: 'utf8',
        });
        return { ok: true, output };
    } catch (error) {
        const { stdout = '', stderr = '' } = error as { stdout?: string; stderr?: string };
        return { ok: false, output: `${stdout}${stderr}` };
    }
}

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
        /ERR_PACKAGE_PATH_NOT_EXPORTED/,
    );
});

test('a release build has no Fake backend in its binary', () => {
    // The binary's exports, loaded straight: FakeBackend is dev-only.
    assert.equal(
        inProject(
            "import { createRequire } from 'node:module'; import { readdirSync } from 'node:fs'; const dir = './node_modules/@pattie/pattie/native/'; const [file] = readdirSync(dir).filter((f) => f.endsWith('.node')); console.log('FakeBackend' in createRequire(import.meta.url)(dir + file))",
        ),
        'false',
    );
});

test('the packed exports name only files the package has', () => {
    const manifest = JSON.parse(
        inProject(
            "import { readFileSync } from 'node:fs'; console.log(readFileSync('node_modules/@pattie/pattie/package.json', 'utf8'))",
        ),
    ) as { exports: Record<string, unknown> };
    assert.deepEqual(Object.keys(manifest.exports), ['.']);
});

test('publint finds nothing wrong with the tarball', () => {
    const { ok, output } = tool('publint', ['run', tarball, '--strict']);
    assert.ok(ok, output);
});

test('the types resolve the same as the code, for ESM consumers', () => {
    // esm-only: a require() of it is Node's require(esm), not a second build.
    const { ok, output } = tool('attw', [tarball, '--profile', 'esm-only', '--format', 'ascii']);
    assert.ok(ok, output);
});

test('without a binary for the platform, importing it is UNSUPPORTED_PLATFORM, naming the ones it has', () => {
    // A copy of the installed package with this platform's binary taken out, as
    // on a platform it has none for (musl, macOS, Windows on Arm).
    const elsewhere = mkdtempSync(join(tmpdir(), 'pattie-unsupported-'));
    writeFileSync(join(elsewhere, 'package.json'), '{ "type": "module", "private": true }');
    const installed = join(elsewhere, 'node_modules', '@pattie', 'pattie');
    cpSync(join(project, 'node_modules', '@pattie', 'pattie'), installed, { recursive: true });
    for (const file of readdirSync(join(installed, 'native'))) {
        if (file.endsWith('.node')) rmSync(join(installed, 'native', file));
    }

    const output = execFileSync(
        process.execPath,
        [
            '--input-type=module',
            '-e',
            "try { await import('@pattie/pattie') } catch (error) { console.log(error.code); console.log(error.message) }",
        ],
        { cwd: elsewhere, encoding: 'utf8' },
    );

    const [code, message = ''] = output.trim().split(/\r?\n/);
    assert.equal(code, 'UNSUPPORTED_PLATFORM');
    assert.match(message, /Windows x64/);
    assert.match(message, /Linux x64 and arm64/);
});
