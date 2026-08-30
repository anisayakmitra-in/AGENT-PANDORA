# Platform support

## CLI

The current beta targets the native Pandora CLI on Windows, macOS, and Linux.
Tagged release builds publish native artifacts for Windows x64, Linux x64, and
macOS Intel and Apple Silicon. The release workflow verifies each artifact
before publishing it.

WSL uses the Linux CLI environment. It is not a separate packaged target.

Source development requires Rust `1.97.1`. Release installation does not
require Rust.

## Desktop app

The current main branch builds Pandora Desktop with Tauri 2 on Windows x64,
Linux x64, macOS Intel, and macOS Apple Silicon. Each package contains a
same-commit `pandora` CLI sidecar. The native launcher rejects a sidecar that
is missing, a symlink, or not a regular executable; release builds do not fall
back to `PATH`.

macOS 26 uses AppKit's supported Liquid Glass Clear material. Older macOS
versions fall back to semantic vibrancy. Linux keeps an opaque application
surface and leaves background effects to the compositor. Windows keeps the
opaque application surface.

The transparent macOS webview requires Tauri's `macOSPrivateApi`. Pandora's
macOS app is therefore a direct-distribution target, not a Mac App Store
target. Stable distribution still requires Apple signing and notarization,
Windows signing, and retained clean-machine release evidence.

Desktop bundle versions resolve from the desktop `package.json`. The release
identity gate requires the desktop npm, lockfile, Cargo, and workspace versions
to match the exact release tag before any package is built. The Windows MSI
upgrade code is pinned so later releases update the same installed product
instead of creating a duplicate application.

The tagged release workflow fails closed for a stable version unless the
Windows certificate, Developer ID Application certificate, Apple notarization
credentials, signing identities, and explicit stable-release approval are all
configured. Signed Windows installers are checked again with `signtool verify`.
Signed macOS app bundles must pass strict `codesign` and Gatekeeper assessment,
and their DMG must pass stapler validation. Every tagged desktop build also
runs the installed-bundle lifecycle check before upload. These controls prove
pipeline readiness; a stable signed release still needs the real credentials
and retained clean-machine evidence.

Use [Desktop accessibility evidence](ACCESSIBILITY.md) for the native Narrator,
VoiceOver, Orca, and scaling protocol. The document records the current Windows
UI Automation checkpoint without presenting it as complete screen-reader
certification.

## Installation verification

Release assets include `checksums.txt`, a signed checksum manifest, an SPDX
SBOM, and GitHub build provenance. The shell and PowerShell CLI installers
verify the artifact checksum before replacing the local binary. Signature
verification can be required with `PANDORA_REQUIRE_SIGNATURE=1` and a
configured Cosign identity.

Each tagged GitHub release includes a `pandora-agent-<version>.tgz` Node/Bun
launcher. It downloads the matching native binary, verifies its checksum,
caches it, and forwards command-line arguments to the `pandora` executable.
It is a downloader and argument forwarder, not a second runtime or authority
boundary.

The launcher is not published to the public npm registry, so
`npm install -g pandora-agent` and equivalent Bun registry installation are
not supported. The immutable first preview retains its original
`o-pandora-cli` asset filename; new release assets use the current package
identity.

## Product boundary

The `2.0.0-beta.7` tag publishes the CLI release line. Desktop packaging was
added after that tag and is verified on the main branch. A desktop package
becomes a supported release only when its tagged workflow publishes the
artifact and records the platform, signing, install, update, and rollback
evidence required by the release policy.
