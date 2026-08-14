# Pandora Agent

Pandora is a governed, CLI-first agent runtime built around:

```text
Parliament → Shadow Council → Harness → Gene → governed execution
```

The `ReferenceMonitor` is the sole authority that can issue effect permits. Genes request work; effect executors perform it only with a valid, scoped, one-shot permit.

## Status

The first named release line is Anubis. The current public preview is
`v2.0.0-anubis.2`; it is not a stable release. See [RELEASES.md](RELEASES.md)
and [platform support](docs/PLATFORMS.md) for the shipped scope and release
requirements.

## Install a tagged CLI release

Use an exact tag. The installer verifies the downloaded native binary against
the release checksum manifest before installation.

```sh
curl -fsSL https://raw.githubusercontent.com/anisayakmitra-in/PANDORA-AGENT/main/scripts/install.sh |
  PANDORA_VERSION=v2.0.0-anubis.2 sh
```

```powershell
$env:PANDORA_VERSION = "v2.0.0-anubis.2"
irm https://raw.githubusercontent.com/anisayakmitra-in/PANDORA-AGENT/main/scripts/install.ps1 | iex
```

The native release assets must exist for the selected tag. Otherwise use the
source-build instructions below.

## Build

Requires Rust `1.97.1`.

```text
cargo test --workspace --lib --tests
cargo run -p pandora-cli -- --version
```

The supported product target is the native CLI on Windows, macOS, and Linux. Desktop, remote execution, mobile, and package marketplace integration remain gated until their release tests and security boundaries exist.
