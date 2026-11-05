# Viaduct driver shell

The thin ACX/KMDF shell around the pure C core in `../core` (ADR 0002). Today it is
a compile probe: `DriverEntry` returns `STATUS_NOT_IMPLEMENTED`. It proves the core
builds and links as kernel code.

## Build (compile only)

Needs Visual Studio's driver tools plus a matching Windows SDK and Driver Kit
(10.0.28000). Use the **64-bit** MSBuild: the 32-bit one makes the kit pick an x86
API-validator helper that can't check x64 drivers.

```bash
"/c/Program Files/Microsoft Visual Studio/18/Community/MSBuild/Current/Bin/amd64/MSBuild.exe" driver/shell/viaduct.vcxproj -p:Configuration=Debug -p:Platform=x64
```

Platforms: `x64`, `ARM64`. Configurations: `Debug`, `Release`. Output lands in
`target/driver/<platform>/<configuration>/viaduct.sys`.

Building never signs, installs or loads anything: `SignMode` is `Off` (no test
certificate is created), and catalog generation and deployment are disabled.

## Kernel-mode notes

- `kernel-crt/` holds kernel-safe `stdint.h` and `stdbool.h`. The kernel C runtime
  has neither, and Visual Studio's versions pull in user-mode headers. The folder is
  first on the include path so the core and ABI header build unchanged.
- The kernel toolset ignores `LanguageStandard_C`; C17 is passed as `/std:c17`.
- The core copies audio samples as raw 32-bit values, so the driver contains no
  floating-point code (nothing for `_fltused` to satisfy on x64).
