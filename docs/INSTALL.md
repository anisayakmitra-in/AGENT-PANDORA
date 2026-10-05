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
`v2.0.0-beta.8`. Do not use a tag until its release page contains the binary
for your platform and its checksum manifest.

## CLI source build

Install Rust `1.97.1`, a platform C toolchain (MSVC Build Tools on Windows or
the equivalent compiler used by your Rust target), clone the repository, and
run:

```sh
cargo build --release -p pandora-cli --locked
cargo run --release -p pandora-cli -- --version
```

To install the CLI from the checkout without npm or Tauri:

```sh
cargo install --path crates/pandora-cli --locked
pandora --version
```

A source build is locally compiled code, not an OS-trust attestation. Gatekeeper,
SmartScreen, antivirus, and enterprise policy may still apply.

## Desktop adapter

Desktop product work is cancelled. `apps/pandora-desktop` stays in the tree for
downstream reuse and audit history, and this repository offers no desktop
install path. Use the CLI source build above or a published CLI installer.

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

For a local Rust build, point the launcher at the binary explicitly instead of
using the release download matrix:

```sh
PANDORA_BIN="$PWD/target/release/pandora" pandora --json doctor
```

PowerShell:

```powershell
$env:PANDORA_BIN = "$PWD\target\release\pandora.exe"
pandora --json doctor
```

`PANDORA_BIN` is an explicit local trust choice. The launcher does not download,
checksum, or replace that binary; use the verified release path when the binary
is not locally built and trusted.

## Support status

The `2.0.0-beta.8` installers support the native CLI on Windows, macOS, and
Linux. The main branch also builds desktop packages for those platforms.
Desktop support remains prerelease until a tagged release publishes the
packages and retains the required clean-machine evidence. No package is
OS-signed on any channel; see
[platform support](PLATFORMS.md) for the integrity guarantees that replace
code signing and for the supported install channels.
