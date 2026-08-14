# Platform support

## CLI

The current preview targets the native Pandora CLI on Windows, macOS, and Linux. Tagged
release builds publish native artifacts for Windows x64, Linux x64, and macOS
Intel and Apple Silicon. The release workflow verifies each artifact before
publishing it.

WSL uses the Linux CLI environment. It is not a separate packaged target.

Source development requires Rust `1.97.1`. Release installation does not
require Rust.

## Installation verification

Release assets include `checksums.txt`, a signed checksum manifest, an SPDX
SBOM, and GitHub build provenance. The shell and PowerShell installers verify
the artifact checksum before replacing the local binary. Signature verification
can be required with `PANDORA_REQUIRE_SIGNATURE=1` and a configured Cosign
identity.

The npm package is a thin Node/Bun launcher. It downloads the matching native
binary, verifies its checksum, caches it, and then forwards the command-line
arguments to that binary.

## Product boundary

The current preview is a CLI release line. No packaged desktop client is part of the current
support claim. A successful workspace build is not evidence that a platform
package exists; the corresponding release artifact and clean-machine checks
must be present first.
