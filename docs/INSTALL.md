# Install Pandora

This guide covers the supported CLI path. The current release line is a
prerelease, so check the [release page](https://github.com/anisayakmitra-in/AGENT-PANDORA/releases)
before installing.

## Published binary

Use a tagged installer on a clean machine. The installer downloads the platform
binary and verifies it against the release checksum manifest.

Unix:

```sh
curl -fsSL https://raw.githubusercontent.com/anisayakmitra-in/AGENT-PANDORA/main/scripts/install.sh | sh
pandora --version
```

Windows PowerShell:

```powershell
irm https://raw.githubusercontent.com/anisayakmitra-in/AGENT-PANDORA/main/scripts/install.ps1 | iex
& "$env:LOCALAPPDATA\Pandora\bin\pandora.exe" --version
```

Pin a published tag with `PANDORA_VERSION`, for example `v2.0.0-beta.7`.
Do not use a tag until its release page contains the binary for your platform
and its checksum manifest.

## Source build

Install Rust `1.97.1`, clone the repository, and run:

```sh
cargo build --release -p pandora-cli --locked
cargo run --release -p pandora-cli -- --version
```

The source build is for development and verification. It is not evidence that
a packaged desktop release exists.

## First run

Start interactive setup and keep provider credentials outside the configuration
file:

```text
pandora setup --interactive
pandora doctor --json
```

Provider endpoints and credential variable names may be stored; credential
values should remain in the environment or an external secret manager.

## npm and Bun

The repository contains a TypeScript launcher package that resolves a verified
native binary. Use it only after the package has been published for the tagged
release. It does not replace the native runtime or create a second permission
boundary.

## Support status

The supported product target is the native CLI on Windows, macOS, and Linux.
The Tauri desktop control surface is source-buildable but is not a supported
packaged release until the release page and cross-platform gates say otherwise.
