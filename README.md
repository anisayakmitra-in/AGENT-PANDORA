# Pandora Agent

Pandora is a governed, CLI-first agent runtime built around:

```text
Parliament → Shadow Council → Harness → Gene → governed execution
```

The `ReferenceMonitor` is the sole authority that can issue effect permits. Genes request work; effect executors perform it only with a valid, scoped, one-shot permit.

## Status

Private development is preparing the first named release line, Anubis. The current build is `2.0.0-alpha.1`; it is not a stable release. See [RELEASES.md](RELEASES.md) for the version and codename policy.

## Build

Requires Rust `1.97.1`.

```text
cargo test --workspace --lib --tests
cargo run -p pandora-cli -- --version
```

The supported product target is the native CLI on Windows, macOS, and Linux. Desktop, remote execution, mobile, and package marketplace integration remain gated until their release tests and security boundaries exist.
