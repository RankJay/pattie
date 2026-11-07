# Viaduct driver shell

The thin ACX/KMDF shell around the pure C core in `core/` (ADR 0002). It only
translates between Windows and the core; every decision is made, and tested, in
the core. Layout (ADR 0010, ADR 0011, ADR 0013):

| Piece | File | Role |
| --- | --- | --- |
| Root ACX device | `src/driver.c` | Installed by `viaduct.inf`; parents the circuit devices |
| Control device `\\.\Viaduct` | `src/control.c` | Owners' commands: one handle per Owner, one buffered IOCTL per command; pins Rings in the caller's context |
| Circuit devices | `src/endpoint.c` | One raw child per Device: create with a stable instance ID from kind + name (ADR 0015), add its circuit once, name it, safe teardown |
| Circuit | `src/circuit.c` | The child's one capture or render circuit: host and bridge pins, float and PCM16 formats |
| Interface values | `src/interfaces.c` | The audio interface values an INF would write, which a runtime child has no INF for |
| Streaming | `src/stream.c` | One timer pass per packet, one copy between the Ring and the ACX packet (PCM16 converted) |
| Tracing | `src/trace.c`, `src/trace.h` | The `Viaduct.Driver` ETW provider: stream format, packets, timer lateness, Gap/Overflow, Device start |

## Build (compile only)

Needs Visual Studio's driver tools plus a matching Windows SDK and Driver Kit
(10.0.28000). Use the **64-bit** MSBuild: the 32-bit one makes the kit pick an x86
API-validator helper that can't check x64 drivers.

```bash
"/c/Program Files/Microsoft Visual Studio/18/Community/MSBuild/Current/Bin/amd64/MSBuild.exe" platforms/windows/driver/viaduct.vcxproj -p:Configuration=Debug -p:Platform=x64
```

Platforms: `x64`, `ARM64`. Configurations: `Debug`, `Release`. Add
`-p:RunCodeAnalysis=true` for Microsoft's driver code analysis (currently clean).
Output lands in `target/driver/<platform>/<configuration>/`, with the stamped INF.

Building never signs, installs or loads anything: `SignMode` is `Off` (no test
certificate is created), and catalog generation and deployment are disabled.

`DriverVer`'s version comes from the build time unless
`-p:ViaductDriverVersion=a.b.c.d` sets it. A release goes through
`platforms/windows/release/make-attestation-cab.ps1 -Version a.b.c.d`, which builds
with that version and packs the `.cab` for Microsoft's attestation signing
(`.ai/VM-STEPS.md`, section H).

On a machine, the driver goes on with `viaduct-cli install <package folder>` and
comes off with `viaduct-cli uninstall`, both as administrator (ADR 0016): install
creates the `Root\Viaduct` device only if there is none, and doesn't replace a
driver of the same or a newer version.

## Kernel-mode notes

- `kernel-crt/` holds kernel-safe `stdint.h` and `stdbool.h`. The kernel C runtime
  has neither, and Visual Studio's versions pull in user-mode headers.
- The kernel toolset ignores `LanguageStandard_C`; C17 is passed as `/std:c17`.
- The core copies audio samples as raw 32-bit values, and converts float to and
  from 16-bit PCM with integer arithmetic on their bit patterns, so the driver
  contains no floating-point code (checked by disassembly).
- In C, `ks.h` only declares the KS GUIDs; `ksguid.lib` defines them.
- Tracing is TraceLogging (ETW) and stays in production builds: it costs nothing
  until a session listens. Events come only from lifecycle callbacks; the timer pass
  just updates counters. The provider must be unregistered on every `DriverEntry`
  failure and on unload, or the system crashes later (`src/driver.c`). Recording
  and reading it: `.ai/VM-STEPS.md`, section G.

## Checked in the test VM (2026-11-07)

First load and the six checks, in a VMware guest with test signing and Driver
Verifier (`.ai/VM-STEPS.md`; what it took: `.ai/VM-FINDINGS.md`, ADR 0013):

1. A runtime child shows up as an audio endpoint: passes.
2. The endpoint is named after the Device and carries the app's audio (the CLI's
   tone in Sound Recorder): passes.
3. Removing a Device removes the endpoint (asynchronously, as documented): passes.
4. A Speaker hands played audio to the app (the CLI's level meter): passes.
5. Killing the app mid-stream: endpoint gone, no `PROCESS_HAS_LOCKED_PAGES`: passes.
6. Driver Verifier, standard flags: no violations; no pool left allocated: passes.

Re-run all six on any change to `src/`, from the "debug-ready" snapshot.
