# Implementation Plan: CLI-First Product Boundary

## Overview

Make the Rust CLI the primary and independently buildable product surface. Keep
core/runtime/service behavior unchanged, preserve the authority chain, and
retire the desktop from the active product/release path without destructively
deleting the existing adapter source. Update documentation and CI so the
repository no longer claims a desktop product that is being cancelled.

## Scope and non-goals

### In scope

- CLI workspace/build/install path as the canonical product path.
- Separation of the npm launcher from the Rust CLI build graph.
- Removal of desktop from active release/product claims and required CI gates.
- Preservation of the desktop source as a non-active downstream adapter.
- Documentation, help text, completions, and release-scope wording.
- Behavior-preserving tests for CLI build, JSON output, and core authority paths.

### Out of scope

- Deleting `apps/pandora-desktop` or its historical code.
- Desktop packaging, signing, notarization, publication, or Tauri builds.
- Changing runtime governance semantics, permissions, effects, approvals, or
  provider behavior.
- Introducing a new runtime, service API, or idempotency protocol as part of
  this boundary refactor.
- Implementing the separate F1 RPC idempotency contract; it remains a reviewed
  follow-up slice.

## Architecture decisions

1. The Rust workspace (`crates/pandora-*`) is the product core. The CLI binary
   is the primary interface and may depend only on workspace crates plus
   standard/toolchain prerequisites.
2. The Tauri app remains source-compatible but is not an active product surface,
   release asset, or required CI gate while cancellation is in effect.
3. The npm package is only a typed launcher. It never builds, signs, or executes
   the native CLI during install; `PANDORA_BIN` is an explicit local override.
4. Documentation must distinguish shipped CLI capability from retained
   downstream/desktop source.
5. No behavior change is introduced by moving a command, changing help text,
   or disabling desktop-only CI/release work.

## Work packets

### W0 — Boundary inventory and plan gate

- Owned files: `tasks/plan.md` and the sibling task list in the same directory.
- Dependencies: none.
- Done: plan, non-goals, and verification commands recorded; no product code
  changed.

### W1 — Canonical CLI build/install contract

- Owned files: root `Cargo.toml` if needed, `crates/pandora-cli/Cargo.toml`,
  CLI CI steps, `docs/INSTALL.md`, `docs/PLATFORMS.md`.
- Dependencies: W0.
- Preserve: all CLI behavior and dependency versions.
- Done: `cargo build/test -p pandora-cli` and `cargo install --path` proof are
  documented and checked without Node/Tauri.

### W2 — npm launcher boundary

- Owned files: `npm/pandora-cli/**`, `scripts/test_npm_launcher.js`,
  `scripts/test_release_notes.py`, CI npm verification steps.
- Dependencies: W0.
- Preserve: checksum-verified release download behavior; no install hooks.
- Done: `PANDORA_BIN`, generated launcher source, drift gate, and no-hook tests
  pass. This packet is already implemented locally as `43eadb1`.

### W3 — Desktop deactivation (non-destructive)

- Owned files: `.github/workflows/ci.yml` desktop job boundaries,
  `.github/workflows/release.yml` desktop publication/verification boundaries,
  `docs/ROADMAP.md`, `docs/WHY_PANDORA.md`, `docs/PRODUCTION.md`,
  `docs/PLATFORMS.md`, `README.md`, `RELEASES.md`, and any desktop-specific
  release documentation.
- Dependencies: W0, W1, W2.
- Preserve: `apps/pandora-desktop` source and historical tags.
- Done: active docs and required gates describe CLI-only delivery; desktop jobs
  no longer block or publish a CLI release; no desktop build is run by the CLI
  verification path.

### W4 — CLI discoverability and structured output

- Owned files: `crates/pandora-cli/src/commands/mod.rs`,
  `crates/pandora-cli/src/commands/completions.rs`, CLI integration tests.
- Dependencies: W1.
- Preserve: existing command behavior and JSON schemas.
- Done: help/completion registry covers the existing CLI surface; every new
  boundary command, if any, has `--json` coverage.

### W5 — Verification and review

- Dependencies: W1–W4.
- Commands:
  - `cargo test -p pandora-cli --locked`
  - `cargo build --release -p pandora-cli --locked`
  - `npm ci --ignore-scripts && npm run build` in `npm/pandora-cli`
  - `node scripts/test_npm_launcher.js`
  - Python repository/docs validation
  - `git diff --check`
  - no desktop build, tag, signing, or publication command.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Removing desktop gates hides useful evidence | Keep source and historical docs; mark the work cancelled rather than deleting it. |
| CI scope change accidentally alters CLI behavior | Keep CLI jobs and tests independent; diff/inspect workflow changes. |
| npm drift gate fails on Windows line endings | Normalize generated output and run the exact CI command locally. |
| Users read “cancel app” as source deletion | State explicitly that the adapter source is retained but inactive. |
| F1 idempotency defect is confused with this refactor | Track it as a separate contract slice; do not mix it into behavior-preserving work. |

## Checkpoints

- After W1/W2: CLI build, install proof, npm tests, and launcher drift pass.
- After W3: documentation/release/CI diff contains no desktop product claim or
  required desktop gate.
- After W4: help/completion parity tests pass.
- Final: full CLI/Python gates pass; no desktop build or publication occurred.
