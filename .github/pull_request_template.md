## What and why

<!-- What this changes, and the reason. Link the issue it closes: "Closes #123". -->

## Checks

<!-- CONTRIBUTING.md, "Before you commit". CI runs them too; tick what you ran locally. -->

- [ ] Tests first: a test fails without this change and passes with it
- [ ] `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] If `pattie_abi.h` or `abi.rs` changed, both changed together and only by addition
- [ ] If the driver changed, checked against the ACX/SdcaVad samples and run on the VM
- [ ] The PR title is a conventional commit: `type(scope): summary`
