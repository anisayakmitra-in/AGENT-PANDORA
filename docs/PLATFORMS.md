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
target.

**Pandora release binaries are not OS-signed.** There is no Authenticode
signature, no Developer ID signature, and no notarization on any release
channel. This is deliberate: the product is a terminal CLI, and it is
distributed through channels that do not require a code signature. Release
integrity comes from three things instead, all published with every release and
verified by the release workflow:

- `checksums.txt`, listing a SHA-256 for every artifact;
- a keyless **cosign signature** over `checksums.txt`
  (`checksums.txt.sig` and `checksums.txt.pem`);
- GitHub **build attestations** from `actions/attest-build-provenance`.

A macOS binary additionally carries the **ad-hoc** signature that Rust's linker
applies by default. That is not a Developer ID signature; it is what allows
Gatekeeper to execute the binary, and the release asserts its presence with
`codesign -dv` while failing if a Developer ID authority is ever claimed.

**Browser-downloaded binaries will still prompt.** A binary fetched through a
browser or double-clicked from `Downloads` may be marked with the
Mark-of-the-Web on Windows or the `com.apple.quarantine` attribute on macOS,
which produces a SmartScreen or Gatekeeper warning regardless of the checksums
above. The supported channels below avoid that by fetching programmatically and
never by double-clicking: `scripts/install.sh` uses `curl` and
`scripts/install.ps1` uses `Invoke-WebRequest`, so neither sets a quarantine
flag. Do not add code that strips those attributes; if a warning appears, the
binary was not obtained through a supported channel.

## Supported distribution channels

| Channel | How it installs | Verifies |
| --- | --- | --- |
| `scripts/install.sh` | `curl` | checksums + cosign signature |
| `scripts/install.ps1` | `Invoke-WebRequest` | checksums + cosign signature |
| npm launcher (`pandora-agent`) | download + checksum | checksums + cosign signature |
| `cargo install --locked` | compiles from source | local build |
| `cargo binstall` | prebuilt binary from the release | checksums |
| Homebrew cask | `brew install` | cask `sha256` |
| Scoop manifest | `scoop install` | manifest `hash` |
| winget manifest | `winget install` | `InstallerSha256` |

The Homebrew, Scoop, and winget manifests are **rendered from the release's own
`checksums.txt`** by `scripts/render_channel_manifests.py` and attached to each
release as assets. A test fails if a channel digest ever drifts from the
published checksum.

### Publishing to Homebrew, Scoop, and winget

These three channels are not yet published. Each needs an external repository
that this project does not own, and each requires a one-time human step. The
rendered manifests are attached to every tagged release, so each step below
starts from a downloadable asset rather than from editing a digest by hand.

**Homebrew** (needs a public tap repository, for example
`github.com/<your-account>/homebrew-pandora`):

1. Create the tap repository with the layout `Formula/pandora.rb`. The default
   Homebrew tap convention expects `Formula/`.
2. Download the release asset `pandora.rb` from the tagged release.
3. Commit it as `Formula/pandora.rb` and push. `brew install --formula
   <your-account>/pandora/pandora` resolves it.
4. Open a pull request to `Homebrew/homebrew-core` once the CLI reaches a stable
   release. Core requires a stable version, an accepted formula style, and
   `brew audit --strict --online` to pass. Homebrew does not require a code
   signature for a CLI cask.

**Scoop** (needs a public bucket repository, conventionally named
`ScoopInstaller/<bucket>`):

1. Create the bucket repository containing a `bucket/` directory.
2. Download the release asset `pandora.json` and commit it as
   `bucket/pandora.json`.
3. Users run `scoop bucket add <bucket>
   https://github.com/<your-account>/ScoopInstaller-<bucket>` then `scoop
   install pandora`.
4. Do not add a `checkver` override by hand; the rendered manifest already
   points `checkver` at `releases/latest/download/checksums.txt`.

**winget** (needs a fork of `microsoft/winget-pkgs`):

1. Fork `microsoft/winget-pkgs` and create a branch off `master`.
2. Download the release asset `AnisayakmitraIn.AGENT-PANDORA.yaml`.
3. Commit it under `manifests/a/AnisayakmitraIn/AGENT-PANDORA/<version>/`.
4. Open a pull request to `microsoft/winget-pkgs`. winget requires the
   manifest to pass `winget validate`; it verifies `InstallerSha256` and does
   not verify Authenticode, so the unsigned binary is not a blocker.

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
between two real published releases can close the release-migration gate.

WiX receives the numeric MSI form of the same release identity. For example,
Pandora `2.0.0-beta.8` is packaged as MSI version `2.0.0.8` because MSI does not
accept named prerelease identifiers. The release identity gate derives and
verifies this mapping.

The tagged release workflow fails closed for a stable version unless explicit
human stable-release approval is configured. It does not require any signing
credential, because no channel OS-signs these binaries. macOS builds assert
their ad-hoc signature with `codesign -dv` and fail if the signature is absent
or if a Developer ID authority is claimed. There is no `signtool verify`, no
notarization, no stapler validation, and no Gatekeeper assessment.
Every tagged desktop build also
runs the installed-bundle lifecycle check before upload. The desktop build
packages the exact native CLI artifact already verified by the release build;
it does not rebuild an independent sidecar. After publication, fresh Linux,
macOS Intel, macOS Apple Silicon, and Windows runners download the native and
desktop assets, authenticate `checksums.txt` with its cosign signature and a
pinned certificate identity, verify the artifacts against it, and verify the
build attestation, then
extract, mount, or administratively unpack the package and run the bounded
launch-and-cleanup lifecycle check. Ephemeral CI runners then exercise the
platform installer contract itself: Debian registers and purges the `pandora`
package, Windows MSI registers into a unique temporary `INSTALLDIR` and
uninstalls it, and macOS copies the app from the DMG into the runner's isolated
user Applications directory before removing it. The verifier refuses this
system-install mode outside an explicit CI environment.
These controls prove pipeline readiness; a stable release still needs retained
real-user installation, update, rollback, and uninstall evidence.

Use [Desktop accessibility evidence](ACCESSIBILITY.md) for the native Narrator,
VoiceOver, Orca, and scaling protocol. The document records the current Windows
UI Automation checkpoint without presenting it as complete screen-reader
certification.

## Installation verification

Release assets include `checksums.txt`, a signed checksum manifest, an SPDX
SBOM, and GitHub build provenance. The shell and PowerShell CLI installers
verify the checksum manifest signature and then the artifact checksum before
replacing the local binary.

Signature verification is on by default and requires
[Pandora's release workflow identity](https://github.com/anisayakmitra-in/AGENT-PANDORA)
to be passed in `PANDORA_COSIGN_IDENTITY`, plus `cosign` on `PATH`:

```sh
PANDORA_COSIGN_IDENTITY='https://github.com/anisayakmitra-in/AGENT-PANDORA/.github/workflows/release.yml@refs/tags/<version>' \
  sh install.sh
```

Verifying the artifact against the manifest is not sufficient on its own — it
only proves the artifact matches a manifest. Without the manifest signature, a
party able to serve both files controls the result. Setting
`PANDORA_REQUIRE_SIGNATURE=0` skips signature verification and is a deliberate
acceptance of that risk; the install still checksums the artifact.

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
