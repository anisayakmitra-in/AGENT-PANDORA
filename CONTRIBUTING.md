# Contributing to Pandora

Pandora is developed in public on `main`. Small, reviewable changes are easier
to validate than broad rewrites.

## Before opening a change

1. Read [Why Pandora?](docs/WHY_PANDORA.md) and [SECURITY.md](SECURITY.md).
2. Explain the user-visible behavior and the authority boundary it touches.
3. Add focused tests for new behavior and failure cases.
4. Keep credentials, private prompts, local paths, and generated build output
   out of commits.

## Local checks

```sh
cargo fmt --all -- --check
cargo check --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --lib --tests --locked
python scripts/validate_repo.py
python scripts/validate_docs.py
```

For desktop changes, also run `npm ci --ignore-scripts` and `npm run build` in
`apps/pandora-desktop`. Native packaging is platform-specific.

## Design rules

- Preserve `Parliament → Shadow Council → Harness → Gene` ownership.
- Keep the ReferenceMonitor as the sole permit issuer.
- Do not let a model, Gene, Skill, Harness, evaluator, or evolution component
  authorize its own effects.
- Keep public contracts versioned and document shipped versus design-only
  behavior.
- Do not copy external projects wholesale; record clean-room inspirations and
  verify licenses before reuse.
