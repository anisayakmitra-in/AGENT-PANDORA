# CLI-first boundary tasks

- [x] W0: Record CLI-first plan, non-goals, and verification gates.
- [x] W1: Verify and document `cargo install --path crates/pandora-cli --locked`
  and the bundled-SQLite C-toolchain prerequisite.
- [x] W2: Add explicit local `PANDORA_BIN` launcher path, TypeScript-owned launcher
  helper, generated-output drift checks, and no-install-hook tests.
- [x] W3: Deactivate desktop from active product/release/CI gates without
  deleting adapter source.
- [x] W3: Update README, roadmap, platform, production, release, and adapter
  documentation to CLI-first status.
- [ ] W4: Audit/fix CLI help and completion parity for the existing command
  surface. (in progress)
- [ ] W5: Run the full CLI, npm, Python, workflow, and documentation gates.

## Checkpoints

- [x] W1/W2: CLI install/build and npm launcher tests pass.
- [x] W3: No active CLI release or required CI job depends on desktop packaging.
- [ ] W4: Help/completions match the CLI command registry.
- [ ] W5: All scoped gates pass; no desktop build, signing, tag, or publication
  occurs.

## Deferred, tracked separately

- F1: explicit RPC `idempotency_key` contract. Desktop product work is
  cancelled, so this is no longer a live desktop defect, but the same constant
  JSON-RPC `id` pattern would break any other client that reused one id for
  approval/resume and other mutations. It changes a public transport contract
  and is out of scope for the behavior-preserving boundary refactor.
