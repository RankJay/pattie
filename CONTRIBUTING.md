# Contributing to Pattie

How to build, test and check a change before it goes in. Report security issues
as [SECURITY.md](SECURITY.md) says, not in a public issue.

## How work is done here

- **Tests first.** Behaviour lands in small slices: write the failing test, make it
pass, and for anything subtle check that breaking the code breaks the test.
- **Logic in the core, Windows in the shell.** New rules about Devices, names or the
Ring go in `core/src/` with a host test in `core/tests/`. The shell stays thin.
- **The ABI is a contract.** Change `pattie_abi.h` and `abi.rs` together; the
`abi_mirror` test fails if they drift. Within major 1, only add (a new minor),
and use the reserved spare bytes only for what ADR 0017 placed there.
- **Check driver changes against Microsoft's samples** (the ACX samples and SdcaVad in
`Windows-driver-samples`) before taking them to the VM. Most hard-won fixes came
from matching what those samples do.

## Before you commit

Run each check yourself, in this order. Each must finish with no warnings.

```bash
cargo fmt --all -- --check
```

```bash
cargo clippy --workspace --all-targets -- -D warnings
```

```bash
cargo test --workspace
```

If you changed dependencies (`Cargo.toml` or `Cargo.lock`), check them
(`deny.toml` lists the licences and sources allowed):

```bash
cargo deny check
```

If you touched the driver, build it too (warnings are errors there), with Microsoft's
code analysis:

```bash
"/c/Program Files/Microsoft Visual Studio/18/Community/MSBuild/Current/Bin/amd64/MSBuild.exe" driver/pattie.vcxproj -p:Configuration=Release -p:Platform=x64 -p:RunCodeAnalysis=true
```

## Sign your commits

Every commit carries a sign-off line, which certifies the
[Developer Certificate of Origin](https://developercertificate.org/): you wrote the
change, or have the right to submit it under the project's licence. `git commit -s`
adds it:

```text
Signed-off-by: Your Name <you@example.com>
```

## Pull requests and branch rules

Changes reach `main` through a pull request, squashed or rebased (no merge commits).
Before it can merge, two checks must pass: `ci-ok` (every job in `ci.yml`) and
`pr-title`. The title must be a conventional commit, `type(scope): summary`, because it
becomes the squash commit's subject. Only the maintainer (@RankJay, the one bypass
actor in `main.json`, by user ID) can push to `main` directly. Release tags (`v*`,
`sdk-v*`) can be created but not moved or deleted, except by repository admins.

These rules are kept as code in `.github/rulesets/`. GitHub doesn't read that folder
on its own, so after changing a file there, an admin applies it:

```bash
.github/rulesets/apply.sh --dry-run
```

```bash
.github/rulesets/apply.sh
```

When you add a job to `ci.yml`, also add it to the `needs` list of `ci-ok`.
Otherwise the rules won't wait for it.

## Repository layout

```
core/
  include/pattie_abi.h      the SDK ↔ driver contract (the only shared header)
  src/                      pure C core: Device table, Ring, clock, PCM16 conversion
                            (run by the Windows driver and, embedded, by the Linux backend)
  tests/conformance/        host tests for the core (Rust harness via bindgen + cc)
  tests/fuzz/               libFuzzer + AddressSanitizer target for the core
crates/
  sdk/                      the Rust SDK, package `pattie` (public API: Device, Writer, Reader,
                            Playout, Converter, Stats, Error, stable_id); the Backends in src/backend/;
                            src/backend/in_process/ runs the C core in-process (Linux, and the
                            contract suite's Fake backend on every OS)
  cli/                      pattie-cli, the test tool: mic, speaker, measure, install, uninstall
  install/                  pattie-install: puts the driver on a machine or takes it off, the
                            engine behind `pattie-cli install` and the MSI's custom action
  msi/                      pattie-msi: the MSI's custom actions (a DLL over pattie-install)
bindings/python/            the Python binding (`pattie` on PyPI), built with maturin
bindings/javascript/        the Node.js binding (`pattie` on npm), built with napi-rs
integrations/pipecat/       the Pipecat transport (`pipecat-pattie` on PyPI)
driver/                     the ACX/KMDF driver: shell sources, pattie.inf, project
  vm/                       scripts for the test VM (package, install, probe, trace)
release/                    make-attestation-cab.ps1 (the .cab for Microsoft signing);
                            make-msi.ps1 and msi/ (pattie.wxs, identity.wxi: the installer); winget/
target/                     build output (driver, VM package, attestation cab, CodeQL)
clippy.toml, deny.toml      banned methods; allowed dependencies (Lint rules, below)
.github/workflows/          CI, nightly checks and releases (they run the checks in this file)
.cargo/mutants.toml         what mutation testing skips, with reasons
LICENSE, NOTICE             the licence (Apache-2.0) and the copyright notice
```

## Lint rules

Where they live, from widest to narrowest:

| Where | Applies to | What it enforces |
| --- | --- | --- |
| Root `Cargo.toml`, `[workspace.lints]` | every crate (each opts in with `[lints] workspace = true`) | The `unsafe` rules below; no `dbg!`, `todo!`, `unimplemented!`; no pointer casts that hide a type or alignment change (`ptr_as_ptr`, `cast_ptr_alignment`, `transmute_ptr_to_ptr`); rustdoc links and HTML must be valid |
| `clippy.toml` | every crate | Methods banned by name, each with its reason: `std::process::exit` (skips destructors, so a live Device isn't removed cleanly) and `std::thread::sleep` (a 1 ms sleep can last ~15.6 ms and starve an audio loop: use the SDK's `Writer::write_paced` or `Reader::read_paced`, which wake on the Device's doorbell). Also: the `unsafe fn` contract rule covers private functions, and tests may unwrap |
| Top of `crates/sdk/src/lib.rs` | the SDK only | It runs inside other people's apps: no `unwrap`/`expect`/`panic!`, no printing, no silently narrowing casts, and every public item is documented and has `Debug` |
| `#![forbid(unsafe_code)]` at the top of a module | `args/`, `clicks/`, `error/`, `stable_id.rs`, `backend/traits.rs`, `audio/` | The compiler guarantees these hold no `unsafe`, so a reviewer can skip them for memory safety |
| `core/tests/conformance/build.rs`, `crates/sdk/build.rs`, `pattie.vcxproj` | the C core (and the driver) | Warnings are errors, plus implicit narrowing and sign changes: `-Wconversion -Wsign-conversion` under clang, C4242/C4244/C4365 under MSVC |

To make an exception in one place, use `#[expect(clippy::…, reason = "…")]` on that
item or statement, never a crate-wide `allow`: the reason is written down, and the
build fails when the exception is no longer needed. The one file-wide exception is
`commands/measure/`, whose WASAPI blocks are each one COM call sequence
(`multiple_unsafe_ops_per_block`, with its reason at the top of the file).

## `unsafe` code

Calling Windows from Rust is always `unsafe`: Windows is C, and Rust can't check the
other side. The rules here:

- keep each `unsafe` block to **one** unsafe operation, with a `// SAFETY:` comment
saying why it's correct (clippy fails the build otherwise:
`undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block`);
- wrap it in a safe function or type (handles freed in `Drop`), so callers never see it;
- shared memory with the kernel (`crates/sdk/src/ring/`) is the one place that
deserves the most care; it follows the Ring protocol exactly, and Miri checks it
(`cargo +nightly miri test -p pattie --lib`).

## Style

- Rust: `rustfmt` defaults, clippy clean.
- C: C17, warnings as errors, no floating point in the driver (the core converts audio
with integer arithmetic on the bit patterns), no allocation on the streaming path.
- Comments say *why*, not what. Docs live next to what they describe: the README for
the project, `driver/README.md` for the driver.
