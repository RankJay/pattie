# Pattie

**Named virtual microphones and speakers that any app can create, and every other app
sees as real audio devices.**

```rust
use pattie::{Device, Kind};

let mic = Device::builder()
    .name("Agent Mic")
    .kind(Kind::Microphone)
    .format(48_000, 1)
    .create()?;                 // "Microphone (Agent Mic)" appears in Windows, Zoom, Teams…

let mut tx = mic.writer()?;
tx.write(&samples)?;            // whatever you write, Zoom hears
// drop(mic): the device disappears
```

No VB-CABLE, no admin prompt in your app, no reboot. Today Rust desktop apps that need
a virtual mic (soundboards, routers, AI voice agents) tell their users to install a
third-party cable. Pattie is the missing piece: a kernel driver plus a small Rust SDK
that talks to it over shared memory.

> **Status: v0 on Windows, plus Linux; not released.** The driver, SDK and CLI work end
> to end in a test VM, including a real Zoom call; the Linux backend and the Python
> binding work too. What's left before the v0 gate is an
> attestation-signed driver installed on a clean Windows 11 with Secure Boot on.
> Nothing is published to crates.io yet.

---
## How it works

An app asks the driver for a Device: a name, a kind and a ring buffer (the **Ring**) in
its own memory. The driver locks the Ring and adds an audio endpoint, so Windows and
every app see "Microphone (Agent Mic)". After that, no system call is made per buffer: a
timer in the driver moves 10 ms at a time between the Ring and Windows' audio engine.
When the app drops the Device, or exits, or crashes, the endpoint goes. The rules live in
a pure-C **core** (`core/`), tested on any PC; the Windows layer around it, the **shell**
(`driver/`), stays thin. On Linux the SDK runs the same core inside the app, with
PipeWire nodes instead of a driver.

## What it is, and isn't

| Pattie is | Pattie is not |
| --- | --- |
| A Windows audio driver that turns "create a device named X" into a real endpoint | A capture library (use `cpal`), a DSP or echo-cancellation library |
| A Rust SDK: create a Device, write to its mic or read from its speaker | A meeting bot or recorder |
| A shared-memory transport between your app and the driver | A GUI app |

The same API is meant to work on Linux and macOS too. Windows is done to the v0 gate;
Linux works (ADR 0018: PipeWire, inside the app's own process, no driver to install;
Microphones and Speakers, every Device format, tested on a real PipeWire); macOS comes
later.

## Where it stands

Measured in a VMware Windows 11 guest (test-signed driver, Driver Verifier on):

| Check | Result |
| --- | --- |
| A Device shows up as an input or output in Windows, Zoom and Sound Recorder, named after the Device | ✅ |
| Removing it (or killing the app) removes the endpoint, no crash, no leaked memory | ✅ |
| Same name, same endpoint every time (Zoom remembers your choice) | ✅ |
| Zoom test call, mic and speaker | ✅ 0 dropouts |
| One hour of streaming | ✅ no drift, no build-up |
| Every CPU core at 100% | ✅ no dropouts |
| 8 Devices from 8 apps at once (the v0 maximum) | ✅ |
| CPU cost per stream | under 0.2% of one core |
| Added latency | ✅ ~28 ms (speaker), ~40 ms (mic): a third to a quarter of VB-CABLE's ~109 ms on the same VM. The original goal, under 10 ms, is still ahead (it needs smaller engine periods than Windows' default) |
| Attestation-signed install, Secure Boot on | ⏳ the v0 gate, next |

## Words we use

| Word | Meaning |
| --- | --- |
| **Device** | A virtual mic or speaker an app created. One name, one kind, one Ring. |
| **Kind** | `Microphone` (app → Windows) or `Speaker` (Windows → app). |
| **Owner** | The app handle that created a Device. When it closes, its Devices go. |
| **Ring** | The shared-memory ring buffer for one Device's audio: 32-bit float frames, one sample per channel, at the Device's rate. |
| **Gap** | Frames the driver needed but the Ring didn't have (silence was played). |
| **Overflow** | Frames the app wrote that didn't fit in the Ring (dropped). |
| **Root device** | The one permanent device, `Root\Pattie`, that makes Windows load the driver. |
| **Endpoint** | What Windows and apps see: "Microphone (Agent Mic)". One per Device. |
| **Device format** | A Device's sample rate and channels, fixed when it is created. Samples in a Device are 32-bit float. |
| **Audio format** | How an app's own audio arrives: rate, channels and encoding. The Converter turns it into the Device format. |
| **Stable ID** | Worked out from a Device's kind, name and format: the same Device always gets the same one, so Windows keeps its settings. |
| **Core / Shell** | The pure-C, host-tested rules / the Driver's own code around them, which talks to Windows. |
| **Driver** | What users install, `pattie.sys`: the Shell and the Core compiled together. |

## Getting started

### Prerequisites (Windows 11 x64)

| Tool | For |
| --- | --- |
| Rust (stable, edition 2024) | SDK, CLI, all tests |
| Visual Studio 2026 (v18) with "Desktop development with C++" and its Clang tools | the C compiler for core tests, and `libclang` for bindgen (`.cargo/config.toml` points at it) |
| Windows Driver Kit 10.0.28000 | building the driver only |
| VMware Workstation Pro (free for personal use) | running the driver only |

You don't need the Driver Kit or a VM to work on the SDK, the CLI or the core.

### Build and test (safe on any PC: nothing is installed or loaded)

```bash
cargo build --workspace
```

```bash
cargo test --workspace
```

This runs the core's C logic, the ABI mirror check and the SDK contract tests against
the In-process core (the C core in the test's own process, with the Fake backend). One
test, `windows_without_driver`, expects the driver to be absent, so it fails on a
machine where Pattie is installed; that's by design.

Formatting and lints: see [CONTRIBUTING.md](CONTRIBUTING.md).

### Build the driver

Use the **64-bit** MSBuild (the 32-bit one picks a validator that can't check x64
drivers):

```bash
"/c/Program Files/Microsoft Visual Studio/18/Community/MSBuild/Current/Bin/amd64/MSBuild.exe" driver/pattie.vcxproj -p:Configuration=Release -p:Platform=x64
```

Add `-p:RunCodeAnalysis=true` for Microsoft's driver code analysis. Output goes to
`target/driver/x64/Release/`. Building never signs, installs or loads anything. More in
[driver/README.md](driver/README.md).

## More

- [CONTRIBUTING.md](CONTRIBUTING.md): how work is done here, the checks before a commit,
  the repository layout, the lint rules.
- [SECURITY.md](SECURITY.md): how to report a vulnerability.
- [driver/README.md](driver/README.md): the driver, and testing it in a VM.
- [bindings/python/](bindings/python/README.md), [integrations/pipecat/](integrations/pipecat/README.md): Python, Pipecat.

## Not in v0

Deliberately left out, to reach the gate first: loopback devices, renaming a live
Device, volume and mute, more than 8 Devices, latency under 10 ms (the original
goal, still ahead), 24-bit and rates beyond 44.1 / 48 / 96 kHz, more than 8
channels, installer self-heal (the SDK reconnecting by itself after a driver
update), the macOS backend, bindings for C, Node and Swift.

## License

Licensed under the [Apache License, Version 2.0](LICENSE). See [NOTICE](NOTICE) for the
copyright.
