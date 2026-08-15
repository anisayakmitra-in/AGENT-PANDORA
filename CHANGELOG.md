# Changelog

## Unreleased

- Invalid agent tool arguments now return bounded, actionable feedback so a
  provider can repair the call without exposing raw argument values.
- The terminal TUI can approve or deny a pending effect and resume the exact
  approved task through the existing approval and execution path.
- The TUI bounds its in-memory transcript and task history for long-running
  terminal sessions.
- `pandora harness inspect` resolves canonical IDs from the built-in catalog;
  `coding` remains a compatibility alias for `coding-domain`.

## v2.0.0-alpha.6

This prerelease connects explicitly enabled Skills to agent context without
changing the governed execution model.

### Shipped

- Enabled Skills contribute bounded reference guidance to the agent system
  instruction; disabled and suspended Skills are excluded.
- Skill guidance is size-limited and fails closed rather than being silently
  truncated.
- Skill text is never treated as permission, policy, approval, or script
  execution authority, and is not persisted in the session transcript.

## v2.0.0-alpha.5

This prerelease adds local Skill admission without changing the governed
execution model.

### Shipped

- `pandora skill install <local-skill-directory>` validates and stages one
  local `SKILL.md` package under the configured Skills root.
- Installed Skills preserve their source, reject duplicate IDs and symlinks,
  and start disabled until explicitly enabled.
- Shell completions and CLI documentation include the install command.

## v2.0.0-alpha.4

This prerelease adds interactive terminal clients without changing the
governed execution model.

### Shipped

- `pandora chat` provides a line-oriented interactive agent session.
- `pandora tui` provides a full-screen terminal client with task history,
  session status, transcript clearing, and clean terminal restoration.
- Both clients reuse the existing AgentLoop, session scope, approval records,
  and governed effect path.

## v2.0.0-alpha.3

This prerelease extends the CLI foundation without changing the governed
execution model.

### Shipped

- The built-in `core-source` Source Harness appears in discovery and binds the
  `pandora-runtime` constitutional service without adding a runnable Gene.
- `pandora session inspect` reports scoped session metadata and bounded event
  counts without exposing stored event payloads.
- Shell completion keeps session subcommands under `pandora session` across
  Bash, Fish, PowerShell, and Zsh.

## v2.0.0-alpha.2

This prerelease updates the CLI foundation without changing the governed
execution model.

### Shipped

- Named provider profiles with one-run provider selection and isolated
  credential environment-variable names.
- Bounded agent transcripts with session resume and one-shot continuation of
  approved pending actions.
- Atomic private configuration writes that preserve the previous valid file on
  replacement failure.
- A public `pandora-agent` npm/Bun launcher that safely replaces stale cached
  binaries on Windows and forwards to verified native artifacts.
- GitHub Actions workflows refreshed to current action runtimes for the
  Windows, macOS, and Linux release checks.

This release remains CLI-only and is a prerelease. Native artifacts are valid
only when the tagged release workflow and its checksums, signature, SBOM, and
provenance assets are present.

## v2.0.0-alpha.1

This is the first plain-SemVer Pandora Agent CLI prerelease.

### Shipped

- Parliament, Shadow Council, Harness, and Gene contracts with one governed execution path.
- Reference-monitor permits, one-shot approvals, effect receipts, and workspace-scoped executors.
- Coding Domain Harness operations for reading, searching, reviewing, patching, and verification.
- Bounded agent mode with strict tool schemas, turn and tool budgets, provider continuation, and session events.
- OpenAI-compatible provider configuration, connectivity testing, structured-output validation, and repair handling.
- SQLite-backed sessions, configuration migration, shell completions, diagnostics, verified updates, rollback, and uninstall.
- Native CLI release workflow for Windows x64, Linux x64, and macOS Intel and Apple Silicon.
- Checksum verification, optional Cosign verification, SPDX SBOM generation, and GitHub build provenance.

### Scope

- This prerelease supports the CLI only.
- Read-only work can run without approval. Filesystem writes and verification processes require an exact approval before execution.
- Skills are local, disabled by default, and cannot execute scripts outside the governed ToolEngine path.
- Desktop, mobile, remote execution, and marketplace clients are outside this release.

This release is a prerelease. Its native artifacts are valid only when the tagged
release workflow and its platform checks have completed successfully.
