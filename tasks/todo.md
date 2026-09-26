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
- [x] W4: Audit/fix CLI help and completion parity for the existing command
  surface. The dispatch table, `ROOT_COMMANDS`, the usage synopsis, and all
  four completion scripts are checked against each other by
  `completion_scripts_cover_the_public_command_surface`, and every command
  added since (`operations`, `graph <kind> show|remove`, `backup recovery`)
  was added to all five surfaces together.
- [x] W5: Run the full CLI, npm, Python, workflow, and documentation gates.
  fmt, workspace clippy on all targets, the full workspace test suite, `npm ci`
  and `npm run build`, the generated-`lib` drift check, the npm launcher and
  TypeScript client tests, the Python script suite, `validate_repo.py`, and
  `validate_docs.py` all pass. CI, Security, Parser fuzz smoke, and CodeQL are
  green on the same commit, and the desktop job stays skipped because
  `PANDORA_DESKTOP_CI` is unset. No tag, release, signing, or publication
  occurred.

## Checkpoints

- [x] W1/W2: CLI install/build and npm launcher tests pass.
- [x] W3: No active CLI release or required CI job depends on desktop packaging.
- [x] W4: Help/completions match the CLI command registry.
- [x] W5: All scoped gates pass; no desktop build, signing, tag, or publication
  occurs.

## Deferred, tracked separately

- F1: explicit RPC `idempotency_key` contract. **Not started.** Desktop product
  work is cancelled, so this is no longer a live desktop defect, but the same
  constant JSON-RPC `id` pattern would break any other client that reused one id
  for approval/resume and other mutations. It changes a public transport
  contract and is out of scope for the behavior-preserving boundary refactor.
  Note that `DurableRpcLedger` and `mcp call --idempotency-key` already exist;
  what is missing is the explicit key on the F1 approval/resume contract
  itself, which is a transport change and needs its own reviewed slice.
