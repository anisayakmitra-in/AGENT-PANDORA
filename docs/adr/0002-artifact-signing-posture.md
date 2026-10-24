# ADR 0002: Native CLI Artifact Signing and Release Integrity Posture

Status: accepted
Deciders: Pandora Core Team
Date: 2026-10-24
Consults: GitHub Issue #3

## Context

GitHub Issue #3 originally proposed introducing native platform code signing
for Pandora artifacts, specifically Windows Authenticode signing, Apple
Developer ID signing, and macOS notarization. At the time the issue was created,
Pandora included a desktop client interface.

On 2026-10-05, the desktop client interface was deleted from the repository.
Pandora is now exclusively a native terminal CLI and headless service daemon
operating across Windows (x86_64), Linux (x86_64), and macOS (x86_64, aarch64).

Traditional platform signing mechanisms (Authenticode, Apple Developer ID)
introduce several operational challenges and security trade-offs for open-source
CLI tools:
1. Platform certificate management requires storing private keys or hardware
   security module (HSM) secrets within CI environments, presenting credential
   exfiltration risks.
2. Apple Developer ID certificates and notarization require annual paid
   subscriptions, which conflicts with decentralized and reproducible open-source
   build pipelines.
3. Package managers commonly used for CLI distribution (Homebrew, Scoop, winget,
   cargo-binstall, npm launcher) verify package integrity via cryptographic
   digests rather than operating-system code signatures.

## Decision

We formally decline platform-level OS code signing (Authenticode, Developer ID,
notarization) and resolve GitHub Issue #3 under the following policy:

1. **CLI-Only Scope:**
   Releases are strictly scoped to native CLI binaries and headless service
   executables. No desktop GUI artifacts are built, packaged, or signed.

2. **Multi-Layered Cryptographic Integrity:**
   In place of platform certificates, release integrity is established through
   three verifiable mechanisms:
   - **SHA-256 Digest Manifest (checksums.txt):** Every release artifact is
     hashed and cataloged.
   - **Keyless Sigstore Cosign Signature:** The checksums.txt file is signed
     using keyless OpenID Connect (OIDC) Cosign (checksums.txt.sig and
     checksums.txt.pem), cryptographically attesting that the manifest was
     produced by the official GitHub Actions workflow from this repository.
   - **GitHub SLSA Build Provenance Attestations:** Built binaries carry
     SLSA-compliant provenance generated via ctions/attest-build-provenance.

3. **macOS Ad-Hoc Signing:**
   macOS binaries carry the default ad-hoc linker signature required by macOS
   Gatekeeper for arm64 execution. The release pipeline asserts the presence of
   an ad-hoc signature while strictly failing if a third-party Developer ID is
   claimed.

4. **Programmatic Installation Channels:**
   Supported installation scripts (scripts/install.sh, scripts/install.ps1,
   and pandora-agent npm launcher) download binaries programmatically and
   verify SHA-256 digests and Cosign signatures prior to execution. This avoids
   browser-induced quarantine flags (Mark-of-the-Web, com.apple.quarantine).

5. **Package Manager Integration:**
   Manifest generators (scripts/generate_release_manifests.py and
   scripts/render_channel_manifests.py) derive Homebrew formulae, Scoop bucket
   manifests, and winget YAML installers directly from the verified
   checksums.txt digests.

## Consequences & Resolution of Issue #3

- **GitHub Issue #3 is closed as Resolved / Obsolete:**
  - Desktop signing is obsolete due to the removal of the desktop GUI product.
  - Native CLI signing requirements are satisfied by Sigstore keyless Cosign
    signatures, GitHub build attestations, and SHA-256 digest validation.
- No platform signing secrets (PFX/p12 certificates, notarization API keys) are
  stored or processed in CI workflows, eliminating a critical class of secret
  exposure vulnerabilities.
- If corporate enterprise distribution channels require Authenticode in the
  future, SignPath Foundation signing may be evaluated via an isolated
  post-build approval workflow as outlined in docs/PRODUCTION.md.
