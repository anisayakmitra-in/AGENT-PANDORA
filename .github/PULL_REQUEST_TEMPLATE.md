## Change

Describe the user-visible behavior and the files that implement it.

## Authority impact

- [ ] No effect or permission boundary changed.
- [ ] The ReferenceMonitor, permit, approval, package, or provider boundary changed and is explained here.

## Verification

- [ ] Focused tests pass.
- [ ] `cargo fmt --all -- --check` passes.
- [ ] `cargo check --workspace --locked` passes.
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings` passes.
- [ ] Documentation reflects shipped behavior.

## Safety

- [ ] No credentials, private data, generated artifacts, or unrestricted local paths are included.
- [ ] Security-sensitive changes include a regression test.
