# Platform support

## CLI

The current beta targets the native Pandora CLI on Windows, macOS, and Linux.
Tagged release builds publish native artifacts for Windows x64, Linux x64, and
macOS Intel and Apple Silicon. The release workflow verifies each artifact
before publishing it.

WSL uses the Linux CLI environment. It is not a separate packaged target.

Source development requires Rust `1.97.1`. Release installation does not
require Rust.

## Retained desktop adapter

Desktop product work is cancelled. `apps/pandora-desktop` stays in the tree for
downstream reuse and audit history, and it has no build, signing, or CI path
here. The paragraphs below record how the adapter used to work; they are not a
support claim.

The adapter used Tauri 2 on Windows x64, Linux x64, macOS Intel, and macOS
Apple Silicon, with a same-commit `pandora` CLI sidecar. Its native launcher
rejected a sidecar that was missing, a symlink, or not a regular executable, and
release builds did not fall back to `PATH`.

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

Desktop CI exercises that identity on every native runner with two same-commit,
synthetic stable versions. It uses the operating system's real Debian package,
DMG application-copy, or MSI registration path and proves launch after install,
in-place update, explicit rollback, and final uninstall. The synthetic packages
are never published. This is bounded installer-mechanics evidence; only a drill
between two real signed releases can close the release-migration gate.

WiX receives the numeric MSI form of the same release identity. For example,
Pandora `2.0.0-beta.8` is packaged as MSI version `2.0.0.8` because MSI does not
accept named prerelease identifiers. The release identity gate derives and
verifies this mapping.

The tagged release workflow fails closed for a stable version unless the
Windows certificate, Developer ID Application certificate, Apple notarization
credentials, signing identities, and explicit stable-release approval are all
configured. Signed Windows installers are checked again with `signtool verify`.
Signed macOS app bundles must pass strict `codesign` and Gatekeeper assessment,
and their DMG must pass stapler validation. Every tagged desktop build also
runs the installed-bundle lifecycle check before upload. The desktop build
packages the exact native CLI artifact already verified by the release build;
it does not rebuild an independent sidecar. After publication, fresh Linux,
macOS Intel, macOS Apple Silicon, and Windows runners download the native and
desktop assets, verify both against the published checksum manifest, then
extract, mount, or administratively unpack the package and run the bounded
launch-and-cleanup lifecycle check. Ephemeral CI runners then exercise the
platform installer contract itself: Debian registers and purges the `pandora`
package, Windows MSI registers into a unique temporary `INSTALLDIR` and
uninstalls it, and macOS copies the app from the DMG into the runner's isolated
user Applications directory before removing it. The verifier refuses this
system-install mode outside an explicit CI environment.
These controls prove pipeline readiness; a stable signed release still needs
the real credentials and retained real-user installation, update, rollback,
and uninstall evidence.

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
boundary. For a local Rust build, set `PANDORA_BIN` to the explicit native
binary; the launcher then skips the release download matrix and does not
checksum or replace that file. The release downloader currently supports
Windows x64, Linux x64, macOS Intel, and macOS Apple Silicon; Cargo remains
the source-build path for other targets.

The launcher is not published to the public npm registry, so
`npm install -g pandora-agent` and equivalent Bun registry installation are
not supported. The immutable first preview retains its original
`o-pandora-cli` asset filename; new release assets use the current package
identity.

## Product boundary

The `2.0.0-beta.8` identity is the next CLI release line, and the CLI is the
only active product surface. Desktop packaging, signing, and publication are
cancelled, so the retained adapter source takes part in no release gate this
repository runs.
