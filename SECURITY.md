# Security

Pattie ships a Windows kernel driver, so a bug can crash or compromise the machine it
runs on. Please report a suspected vulnerability privately.

## How to report

Use GitHub's private vulnerability reporting: the **Security** tab of this repository,
then **Report a vulnerability**. Please do not open a public issue for it.

Include what you found, the version or commit, and the steps to reproduce it. A crash
dump or a trace helps for driver issues.

## What is in scope

- the driver (`driver/`) and the C core it runs (`core/`);
- the SDK's shared memory with the driver (`crates/sdk/src/ring/`) and its Commands;
- the installer (`crates/install`, `crates/msi`, `release/`).

Nothing is published yet, so there are no supported releases; reports apply to the
latest commit on the default branch.
