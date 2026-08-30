# Install Pandora

This guide covers the published CLI installer and the desktop source build.
The current release line is a prerelease, so check the
[release page](https://github.com/anisayakmitra-in/AGENT-PANDORA/releases)
before installing an artifact.

## Published CLI binary

Use a tagged installer on a clean machine. The installer downloads the
platform binary and verifies it against the release checksum manifest.

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

Pin a published tag with `PANDORA_VERSION`, for example
`v2.0.0-beta.7`. Do not use a tag until its release page contains the binary
for your platform and its checksum manifest.

## CLI source build

Install Rust `1.97.1`, clone the repository, and run:

```sh
cargo build --release -p pandora-cli --locked
cargo run --release -p pandora-cli -- --version
```

## Desktop source build

Pandora Desktop has no account or login. It starts the same local Pandora
service used by the CLI and keeps service credentials in the native Tauri
layer.

Install Node.js and the native Tauri prerequisites for your platform, then run:

```sh
cd apps/pandora-desktop
npm ci
npm run tauri:build
```

The build stages a same-commit CLI sidecar before packaging. On macOS, use
`./script/build_and_run.sh --verify` for the project build, checks, and app
bundle launch. See [Platform support](PLATFORMS.md) for macOS direct
distribution and current signing limits.

## First CLI run

Start interactive setup and keep provider credentials outside the
configuration file:

```text
pandora setup --interactive
pandora doctor --json
```

Provider endpoints and credential variable names may be stored; credential
values should remain in the encrypted local vault, environment, or an external
secret manager.

## npm and Bun

The repository contains a TypeScript launcher package that resolves a verified
native CLI binary. Use it only after the package has been published for the
tagged release. It does not replace the Rust runtime or create a second
permission boundary.

## Support status

The `2.0.0-beta.7` installers support the native CLI on Windows, macOS, and
Linux. The main branch also builds desktop packages for those platforms.
Desktop support remains prerelease until a tagged release publishes the
packages and retains the required signing and clean-machine evidence.
