# Viaduct driver shell

The thin ACX/KMDF shell around the pure C core in `../core` (ADR 0002). It only
translates between Windows and the core; every decision is made, and tested, in
the core. Layout (ADR 0010, ADR 0011):

| Piece | File | Role |
| --- | --- | --- |
| Root ACX device | `src/driver.c` | Installed by `viaduct.inf`; parents the circuit devices |
| Control device `\\.\Viaduct` | `src/control.c` | Owners' commands: one handle per Owner, one buffered IOCTL per command; pins Rings in the caller's context |
| Circuit devices | `src/endpoint.c` | One raw child per Device, carrying one capture or render circuit; safe teardown |
| Streaming | `src/stream.c` | One timer pass per packet, one copy between the Ring and the ACX packet |

## Build (compile only)

Needs Visual Studio's driver tools plus a matching Windows SDK and Driver Kit
(10.0.28000). Use the **64-bit** MSBuild: the 32-bit one makes the kit pick an x86
API-validator helper that can't check x64 drivers.

```bash
"/c/Program Files/Microsoft Visual Studio/18/Community/MSBuild/Current/Bin/amd64/MSBuild.exe" driver/shell/viaduct.vcxproj -p:Configuration=Debug -p:Platform=x64
```

Platforms: `x64`, `ARM64`. Configurations: `Debug`, `Release`. Add
`-p:RunCodeAnalysis=true` for Microsoft's driver code analysis (currently clean).
Output lands in `target/driver/<platform>/<configuration>/`, with the stamped INF.

Building never signs, installs or loads anything: `SignMode` is `Off` (no test
certificate is created), and catalog generation and deployment are disabled.

## Kernel-mode notes

- `kernel-crt/` holds kernel-safe `stdint.h` and `stdbool.h`. The kernel C runtime
  has neither, and Visual Studio's versions pull in user-mode headers.
- The kernel toolset ignores `LanguageStandard_C`; C17 is passed as `/std:c17`.
- The core copies audio samples as raw 32-bit values, so the driver contains no
  floating-point code.
- In C, `ks.h` only declares the KS GUIDs; `ksguid.lib` defines them.

## Unproven until the test VM

Building proves the code is well-formed, not that Windows accepts it. These are
the first things to check when the driver is first loaded in the VM:

1. Runtime circuit devices: a raw child created with `WdfPdoInitAllocate` and
   `AcxDeviceAddCircuitDevice` shows up as an audio endpoint. Microsoft's sample
   INF registers each endpoint's interfaces and friendly name under fixed names;
   runtime children have no INF of their own. If endpoints don't appear or are
   unnamed, use ADR 0001's fallback: a fixed pool of INF-defined endpoints.
2. The endpoint is named after the Device (the child's device text), so Zoom shows
   "Agent Mic".
3. `AcxDeviceRemoveCircuitDevice` completes asynchronously, as documented.
4. ACX accepts 48 kHz mono 32-bit float, and the engine's packet size divides into
   whole frames.
5. Removing a Device mid-stream: silence, then the endpoint disappears; no
   `PROCESS_HAS_LOCKED_PAGES` when an app is killed while streaming.
6. Driver Verifier stays clean across create, stream, remove and app crash.
