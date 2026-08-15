# Pandora Agent

Pandora is a governed, CLI-first agent runtime built around:

```text
Parliament → Shadow Council → Harness → Gene → governed execution
```

The `ReferenceMonitor` is the sole authority that can issue effect permits. Genes request work; effect executors perform it only with a valid, scoped, one-shot permit.

## Status

The active prerelease is `2.0.0-alpha.1` and is CLI-only. Existing legacy
preview tags remain immutable for compatibility. New releases use plain SemVer
tags. See [RELEASES.md](RELEASES.md), [CHANGELOG.md](CHANGELOG.md), and
[platform support](docs/PLATFORMS.md) for the shipped scope and release gates.

## Install a tagged CLI release

Use the exact published tag. The installer verifies the downloaded native
binary against the release checksum manifest before installation.

```sh
curl -fsSL https://raw.githubusercontent.com/anisayakmitra-in/PANDORA-AGENT/main/scripts/install.sh |
  PANDORA_VERSION=v2.0.0-alpha.1 sh
```

```powershell
$env:PANDORA_VERSION = "v2.0.0-alpha.1"
irm https://raw.githubusercontent.com/anisayakmitra-in/PANDORA-AGENT/main/scripts/install.ps1 | iex
```

The example remains unavailable until that tag has published native assets.
Until then, use the source-build instructions below.

## Build

Requires Rust `1.97.1`.

```text
cargo test --workspace --lib --tests
cargo run -p pandora-cli -- --version
```

The supported product target is the native CLI on Windows, macOS, and Linux. Desktop, remote execution, mobile, and package marketplace integration remain gated until their release tests and security boundaries exist.
