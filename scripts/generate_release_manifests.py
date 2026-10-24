"""Generate Homebrew bottle formula, Scoop bucket JSON, and winget YAML manifests.

Validates the release against checksums.txt and optional cosign signatures.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

_DIGEST_RE = re.compile(r"^[0-9a-fA-F]{64}$")
_RELEASE_TAG_RE = re.compile(
    r"^v[0-9]+\.[0-9]+\.[0-9]+(?:-(?:alpha|beta|rc)\.[0-9]+)?$"
)

REQUIRED_ARTIFACTS = (
    "pandora-aarch64-apple-darwin",
    "pandora-x86_64-apple-darwin",
    "pandora-x86_64-unknown-linux-gnu",
    "pandora-x86_64-pc-windows-msvc.exe",
)


class ManifestGenerationError(ValueError):
    """Raised when release manifest generation or validation fails."""


def parse_checksum_file(path: Path) -> dict[str, str]:
    """Parse a standard sha256 checksums file into a map of filename -> hex digest."""
    if not path.is_file():
        raise ManifestGenerationError(f"checksum file not found: {path}")

    text = path.read_text(encoding="utf-8")
    checksums: dict[str, str] = {}
    for line_num, raw_line in enumerate(text.splitlines(), start=1):
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) != 2:
            raise ManifestGenerationError(
                f"malformed line {line_num} in checksum file: {raw_line}"
            )
        digest, filename = parts[0].lower(), parts[1].lstrip("*").strip()
        if not _DIGEST_RE.fullmatch(digest):
            raise ManifestGenerationError(
                f"invalid SHA-256 digest '{digest}' on line {line_num}"
            )
        clean_filename = Path(filename).name
        checksums[clean_filename] = digest
        if filename != clean_filename:
            checksums[filename] = digest

    if not checksums:
        raise ManifestGenerationError(f"checksum file is empty: {path}")
    return checksums


def verify_cosign_signature(
    manifest_path: Path,
    signature_path: Path,
    certificate_path: Path,
    identity: str,
    issuer: str = "https://token.actions.githubusercontent.com",
) -> bool:
    """Verify keyless cosign signature over the checksum manifest."""
    cosign = shutil.which("cosign")
    if cosign is None:
        raise ManifestGenerationError(
            "cosign executable is not found in PATH; cannot verify signature"
        )

    cmd = [
        cosign,
        "verify-blob",
        str(manifest_path),
        "--certificate",
        str(certificate_path),
        "--signature",
        str(signature_path),
        "--certificate-identity",
        identity,
        "--certificate-oidc-issuer",
        issuer,
    ]
    proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout or "").strip()
        raise ManifestGenerationError(
            f"cosign signature verification failed (code {proc.returncode}): {detail}"
        )
    return True


def validate_release_checksums(
    checksums_path: Path,
    *,
    require_all_targets: bool = True,
    require_cosign: bool = False,
    identity: str | None = None,
    issuer: str = "https://token.actions.githubusercontent.com",
) -> dict[str, str]:
    """Validate checksum manifest integrity and optional cosign signature."""
    checksums = parse_checksum_file(checksums_path)

    if require_all_targets:
        missing = [a for a in REQUIRED_ARTIFACTS if a not in checksums]
        if missing:
            raise ManifestGenerationError(
                f"checksums file is missing required release artifacts: {', '.join(missing)}"
            )

    sig_path = checksums_path.with_name(checksums_path.name + ".sig")
    cert_path = checksums_path.with_name(checksums_path.name + ".pem")

    has_sig = sig_path.is_file() and cert_path.is_file()
    if has_sig or require_cosign:
        if not has_sig:
            raise ManifestGenerationError(
                f"cosign signature or certificate missing for {checksums_path}"
            )
        resolved_identity = identity or "https://github.com/anisayakmitra-in/AGENT-PANDORA"
        verify_cosign_signature(
            checksums_path, sig_path, cert_path, identity=resolved_identity, issuer=issuer
        )

    return checksums


def generate_homebrew_formula(
    version: str,
    tag: str,
    checksums: dict[str, str],
) -> str:
    """Generate Homebrew Formula Ruby file content."""
    arm_mac_sha = checksums.get("pandora-aarch64-apple-darwin", "")
    intel_mac_sha = checksums.get("pandora-x86_64-apple-darwin", "")
    linux_sha = checksums.get("pandora-x86_64-unknown-linux-gnu", "")

    return f"""class Pandora < Formula
  desc "Terminal-first agent runtime"
  homepage "https://github.com/anisayakmitra-in/AGENT-PANDORA"
  version "{version}"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download/{tag}/pandora-aarch64-apple-darwin"
      sha256 "{arm_mac_sha}"
    else
      url "https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download/{tag}/pandora-x86_64-apple-darwin"
      sha256 "{intel_mac_sha}"
    end
  end

  on_linux do
    if Hardware::CPU.intel?
      url "https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download/{tag}/pandora-x86_64-unknown-linux-gnu"
      sha256 "{linux_sha}"
    end
  end

  def install
    binary_name = Hardware::CPU.arm? ? "pandora-aarch64-apple-darwin" : (OS.mac? ? "pandora-x86_64-apple-darwin" : "pandora-x86_64-unknown-linux-gnu")
    if File.exist?(binary_name)
      bin.install binary_name => "pandora"
    else
      bin.install Dir["pandora*"].first => "pandora"
    end
  end

  test do
    assert_match "pandora", shell_output("#{{bin}}/pandora --version")
  end
end
"""


def generate_scoop_manifest(
    version: str,
    tag: str,
    checksums: dict[str, str],
) -> dict[str, object]:
    """Generate Scoop bucket JSON dictionary."""
    win_sha = checksums.get("pandora-x86_64-pc-windows-msvc.exe", "")
    return {
        "version": version,
        "description": "Terminal-first agent runtime",
        "homepage": "https://github.com/anisayakmitra-in/AGENT-PANDORA",
        "license": "MIT",
        "depends": "git",
        "architecture": {
            "64bit": {
                "url": f"https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download/{tag}/pandora-x86_64-pc-windows-msvc.exe",
                "hash": win_sha,
                "bin": [
                    [
                        "pandora-x86_64-pc-windows-msvc.exe",
                        "pandora.exe",
                    ]
                ],
            }
        },
        "checkver": "https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/latest/download/checksums.txt",
        "autoupdate": {
            "architecture": {
                "64bit": {
                    "url": f"https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download/{tag}/pandora-x86_64-pc-windows-msvc.exe"
                }
            }
        },
    }


def generate_winget_version_manifest(version: str) -> str:
    """Generate winget version YAML manifest."""
    return f"""# yaml-language-server: $schema=https://aka.ms/winget-manifest.version.1.6.0.schema.json
PackageIdentifier: AnisayakmitraIn.AGENT-PANDORA
PackageVersion: {version}
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
"""


def generate_winget_installer_manifest(
    version: str,
    tag: str,
    checksums: dict[str, str],
) -> str:
    """Generate winget installer YAML manifest."""
    win_sha = checksums.get("pandora-x86_64-pc-windows-msvc.exe", "")
    return f"""# yaml-language-server: $schema=https://aka.ms/winget-manifest.installer.1.6.0.schema.json
PackageIdentifier: AnisayakmitraIn.AGENT-PANDORA
PackageVersion: {version}
Platform:
  - Windows.Desktop
InstallModes:
  - silent
  - interactive
Installers:
  - Architecture: x64
    InstallerType: portable
    InstallerUrl: https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download/{tag}/pandora-x86_64-pc-windows-msvc.exe
    InstallerSha256: {win_sha}
    Commands:
      - pandora
ManifestType: installer
ManifestVersion: 1.6.0
"""


def generate_winget_default_locale_manifest(version: str) -> str:
    """Generate winget default locale YAML manifest."""
    return f"""# yaml-language-server: $schema=https://aka.ms/winget-manifest.defaultLocale.1.6.0.schema.json
PackageIdentifier: AnisayakmitraIn.AGENT-PANDORA
PackageVersion: {version}
PackageLocale: en-US
Publisher: ANISAYAK MITRA
PackageName: AGENT-PANDORA
License: MIT
ShortDescription: Terminal-first agent runtime
Description: High-performance terminal-first autonomous agent platform and runtime.
Moniker: pandora
Tags:
  - agent
  - ai
  - cli
  - runtime
ManifestType: defaultLocale
ManifestVersion: 1.6.0
"""


def generate_release_manifests(
    output_dir: Path,
    checksums_path: Path,
    tag: str,
    *,
    validate: bool = True,
    require_all_targets: bool = True,
    require_cosign: bool = False,
    identity: str | None = None,
) -> dict[str, Path]:
    """Generate all package manager release manifests and return paths written."""
    clean_tag = tag if tag.startswith("v") else f"v{tag}"
    version = clean_tag.lstrip("v")

    if validate:
        checksums = validate_release_checksums(
            checksums_path,
            require_all_targets=require_all_targets,
            require_cosign=require_cosign,
            identity=identity,
        )
    else:
        checksums = parse_checksum_file(checksums_path)

    output_dir.mkdir(parents=True, exist_ok=True)
    results: dict[str, Path] = {}

    # 1. Homebrew Formula
    formula_dir = output_dir / "Formula"
    formula_dir.mkdir(parents=True, exist_ok=True)
    formula_file = formula_dir / "pandora.rb"
    formula_content = generate_homebrew_formula(version, clean_tag, checksums)
    formula_file.write_text(formula_content, encoding="utf-8")
    results["homebrew_formula"] = formula_file

    # Also root-level convenience formula
    (output_dir / "pandora.rb").write_text(formula_content, encoding="utf-8")

    # 2. Scoop Bucket JSON
    bucket_dir = output_dir / "bucket"
    bucket_dir.mkdir(parents=True, exist_ok=True)
    scoop_file = bucket_dir / "pandora.json"
    scoop_data = generate_scoop_manifest(version, clean_tag, checksums)
    scoop_file.write_text(
        json.dumps(scoop_data, indent=2) + "\n", encoding="utf-8"
    )
    results["scoop_manifest"] = scoop_file

    # Also root-level convenience scoop
    (output_dir / "pandora.json").write_text(
        json.dumps(scoop_data, indent=2) + "\n", encoding="utf-8"
    )

    # 3. Winget Manifests
    winget_dir = output_dir / "winget"
    winget_dir.mkdir(parents=True, exist_ok=True)
    ver_file = winget_dir / "manifest.version.yaml"
    inst_file = winget_dir / "manifest.installer.yaml"
    loc_file = winget_dir / "manifest.defaultLocale.yaml"

    ver_content = generate_winget_version_manifest(version)
    inst_content = generate_winget_installer_manifest(version, clean_tag, checksums)
    loc_content = generate_winget_default_locale_manifest(version)

    ver_file.write_text(ver_content, encoding="utf-8")
    inst_file.write_text(inst_content, encoding="utf-8")
    loc_file.write_text(loc_content, encoding="utf-8")

    results["winget_version"] = ver_file
    results["winget_installer"] = inst_file
    results["winget_default_locale"] = loc_file

    # Also root-level convenience winget
    (output_dir / "manifest.version.yaml").write_text(ver_content, encoding="utf-8")
    (output_dir / "manifest.installer.yaml").write_text(
        inst_content, encoding="utf-8"
    )
    (output_dir / "manifest.defaultLocale.yaml").write_text(
        loc_content, encoding="utf-8"
    )

    return results


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Generate Homebrew, Scoop, and winget release manifests from checksums."
    )
    parser.add_argument("tag", help="Release git tag (e.g. v2.0.0-beta.10)")
    parser.add_argument(
        "--checksums",
        "--manifest",
        dest="checksums",
        type=Path,
        required=True,
        help="Path to checksums.txt file",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        required=True,
        help="Directory to write rendered manifests into",
    )
    parser.add_argument(
        "--validate",
        action="store_true",
        default=True,
        help="Validate all required release artifact checksums (default: true)",
    )
    parser.add_argument(
        "--no-validate",
        action="store_false",
        dest="validate",
        help="Skip target completeness validation",
    )
    parser.add_argument(
        "--require-cosign",
        action="store_true",
        default=False,
        help="Enforce cosign signature verification",
    )
    parser.add_argument(
        "--identity",
        default=None,
        help="Expected cosign OIDC certificate identity",
    )

    args = parser.parse_args(argv)

    try:
        results = generate_release_manifests(
            output_dir=args.output_dir,
            checksums_path=args.checksums,
            tag=args.tag,
            validate=args.validate,
            require_cosign=args.require_cosign,
            identity=args.identity,
        )
    except Exception as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    for kind, path in sorted(results.items()):
        print(f"{kind}: {path}")
    print("release manifests successfully generated and validated.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
