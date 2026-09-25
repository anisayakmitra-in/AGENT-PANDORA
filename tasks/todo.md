# CLI-first boundary tasks

- [x] W0: Record CLI-first plan, non-goals, and verification gates.
- [x] W2: Add explicit local `PANDORA_BIN` launcher path, TypeScript-owned launcher helper, generated-output drift checks, and no-install-hook tests.
- [ ] W1: Verify and document `cargo install --path crates/pandora-cli --locked` and the bundled-SQLite C-toolchain prerequisite.
- [ ] W3: Deactivate desktop from active product/release/CI gates without deleting adapter source.
- [ ] W3: Update README, roadmap, platform, production, and release documentation to CLI-first status.
- [ ] W4: Audit/fix CLI help and completion parity for the existing command surface.
- [ ] W5: Run CLI, npm, Python, workflow, and documentation verification gates.
- [ ] Separate follow-up: F1 explicit RPC idempotency-key contract; not part of this behavior-preserving boundary refactor.

## Checkpoints

- [ ] W1/W2: CLI install/build and npm launcher tests pass.
- [ ] W3: No active CLI release or required CI job depends on desktop packaging.
- [ ] W4: Help/completions match the CLI command registry.
- [ ] W5: All scoped gates pass; no desktop build, signing, tag, or publication occurs.
