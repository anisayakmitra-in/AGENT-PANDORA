from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path
from typing import Sequence

try:
    from .installer_contract import expected_checksum, parse_checksums
    from .platform_signing_receipt import (
        SigningReceiptError,
        receipt_digest,
        read_platform_signing_receipt,
        validate_platform_signing_receipt,
    )
except ImportError:
    from installer_contract import expected_checksum, parse_checksums
    from platform_signing_receipt import (
        SigningReceiptError,
        receipt_digest,
        read_platform_signing_receipt,
        validate_platform_signing_receipt,
    )


_RELEASE_TAG = re.compile(
    r"^v[0-9]+\.[0-9]+\.[0-9]+(?:-(?:alpha|beta|rc)\.[0-9]+)?$"
)
_REQUIRED_FILES = (
    "checksums.txt",
    "checksums.txt.sig",
    "checksums.txt.pem",
    "pandora-cargo-metadata.json",
    "pandora.spdx.json",
)
_SIGNATURE_FILES = {"checksums.txt.sig", "checksums.txt.pem"}
_METADATA_FILES = {"checksums.txt", *_SIGNATURE_FILES, "release-evidence.json"}
_SCOPES = {"full", "cli-only"}
_GIT_COMMIT = re.compile(r"^(?:[0-9a-f]{40}|[0-9a-f]{64})$")
_VENDOR_PLATFORMS = ("windows", "macos")


class ReleaseEvidenceError(ValueError):
    pass


def platform_signing_required(tag: str) -> bool:
    version = tag[1:]
    return "-rc." in version or "-" not in version


def stable_rollback_state(tag: str) -> str:
    if _RELEASE_TAG.fullmatch(tag) is None:
        raise ReleaseEvidenceError(f"invalid release tag: {tag}")
    version = tag[1:]
    if "-" in version:
        return "not_applicable_prerelease"
    patch = int(version.split(".")[2])
    if patch == 0:
        return "pending_first_patch"
    return "requires_post_publication_verification"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _release_files(dist: Path) -> dict[str, Path]:
    if not dist.is_dir():
        raise ReleaseEvidenceError(f"release directory does not exist: {dist}")

    files: dict[str, Path] = {}
    for path in sorted(dist.iterdir(), key=lambda candidate: candidate.name):
        if path.is_symlink() or not path.is_file():
            raise ReleaseEvidenceError(f"release directory contains a non-file: {path.name}")
        if path.name in files:
            raise ReleaseEvidenceError(f"duplicate release filename: {path.name}")
        files[path.name] = path
    return files


def _require_files(files: dict[str, Path], *, desktop_required: bool) -> None:
    for name in _REQUIRED_FILES:
        path = files.get(name)
        if path is None or path.stat().st_size == 0:
            raise ReleaseEvidenceError(f"required release evidence file is missing or empty: {name}")

    native = [
        name
        for name in files
        if name.startswith("pandora-")
        and name not in {"pandora-cargo-metadata.json", "pandora.spdx.json"}
    ]
    desktop = [name for name in files if name.startswith("desktop-")]
    if not native:
        raise ReleaseEvidenceError("release evidence has no native CLI artifact")
    if desktop_required and not desktop:
        raise ReleaseEvidenceError("release evidence has no desktop artifact")
    if not desktop_required and desktop:
        raise ReleaseEvidenceError(
            "cli-only release evidence must not contain desktop artifacts"
        )


def _platform_for_artifact(name: str) -> str | None:
    if name in {"pandora-cargo-metadata.json", "pandora.spdx.json"}:
        return None
    if name.startswith("desktop-windows-"):
        return "windows"
    if name.startswith("desktop-macos-"):
        return "macos"
    if name.startswith("pandora-") and "windows" in name:
        return "windows"
    if name.startswith("pandora-") and ("apple" in name or "macos" in name):
        return "macos"
    return None


def _validate_commit(commit: str | None, *, required: bool) -> str | None:
    if commit is None:
        if required:
            raise ReleaseEvidenceError("platform signing evidence requires a commit")
        return None
    if _GIT_COMMIT.fullmatch(commit) is None:
        raise ReleaseEvidenceError("release evidence commit is invalid")
    return commit


def _load_and_validate_signing_receipts(
    tag: str,
    dist: Path,
    files: dict[str, Path],
    *,
    commit: str | None,
    receipt_paths: Sequence[Path],
    signing_required: bool,
) -> list[dict[str, object]]:
    if signing_required and not receipt_paths:
        raise ReleaseEvidenceError("platform signing receipts are required")
    if receipt_paths and commit is None:
        raise ReleaseEvidenceError("platform signing receipts require a commit")

    required_names = {
        name for name in files if (platform := _platform_for_artifact(name)) is not None
    }
    by_artifact: dict[str, dict[str, object]] = {}
    for receipt_path in receipt_paths:
        try:
            receipt, receipt_file_sha256 = read_platform_signing_receipt(receipt_path)
        except SigningReceiptError as error:
            raise ReleaseEvidenceError(str(error)) from error
        if not isinstance(receipt, dict):
            raise ReleaseEvidenceError(f"signing receipt is not an object: {receipt_path}")
        artifact = receipt.get("artifact")
        if not isinstance(artifact, dict) or type(artifact.get("path")) is not str:
            raise ReleaseEvidenceError(f"signing receipt has no artifact path: {receipt_path}")
        artifact_name = artifact["path"]
        platform = _platform_for_artifact(artifact_name)
        if platform is None:
            raise ReleaseEvidenceError(
                f"signing receipt references a non-vendor artifact: {artifact_name}"
            )
        if receipt.get("platform") != platform:
            raise ReleaseEvidenceError(
                f"signing receipt platform does not match artifact: {artifact_name}"
            )
        if artifact_name in by_artifact:
            raise ReleaseEvidenceError(f"duplicate signing receipt for artifact: {artifact_name}")
        if artifact_name not in files:
            raise ReleaseEvidenceError(
                f"signing receipt references an artifact outside the release: {artifact_name}"
            )
        try:
            validated = validate_platform_signing_receipt(
                receipt,
                dist / artifact_name,
                expected_tag=tag,
                expected_commit=commit,
                require_verified=True,
            )
        except SigningReceiptError as error:
            raise ReleaseEvidenceError(str(error)) from error
        by_artifact[artifact_name] = {
            "path": receipt_path.name,
            "artifact": artifact_name,
            "platform": platform,
            "file_sha256": receipt_file_sha256,
            "receipt_sha256": receipt_digest(validated),
        }

    if signing_required:
        missing = sorted(required_names - set(by_artifact))
        if missing:
            raise ReleaseEvidenceError(
                "platform signing receipts are incomplete: " + ", ".join(missing)
            )
        missing_platforms = [
            platform
            for platform in _VENDOR_PLATFORMS
            if not any(entry["platform"] == platform for entry in by_artifact.values())
        ]
        if missing_platforms:
            raise ReleaseEvidenceError(
                "platform signing receipts have no evidence for: "
                + ", ".join(missing_platforms)
            )
    return [by_artifact[name] for name in sorted(by_artifact)]


def build_release_evidence(
    tag: str,
    dist: Path,
    *,
    scope: str = "full",
    commit: str | None = None,
    signing_receipts: Sequence[Path] = (),
) -> dict[str, object]:
    if _RELEASE_TAG.fullmatch(tag) is None:
        raise ReleaseEvidenceError(f"invalid release tag: {tag}")
    if type(scope) is not str or scope not in _SCOPES:
        raise ReleaseEvidenceError(f"unsupported release scope: {scope}")
    signing_required = platform_signing_required(tag)
    if scope == "cli-only" and signing_required:
        raise ReleaseEvidenceError(
            "release-candidate and stable releases require full scope"
        )
    normalized_commit = _validate_commit(commit, required=signing_required)
    desktop_required = scope == "full"

    files = _release_files(dist)
    _require_files(files, desktop_required=desktop_required)
    signing_receipt_entries = _load_and_validate_signing_receipts(
        tag,
        dist,
        files,
        commit=normalized_commit,
        receipt_paths=signing_receipts,
        signing_required=signing_required,
    )
    try:
        checksums = parse_checksums(files["checksums.txt"].read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, ValueError) as error:
        raise ReleaseEvidenceError(f"invalid checksum manifest: {error}") from error

    artifacts: list[dict[str, object]] = []
    for name, path in files.items():
        if name in _METADATA_FILES:
            continue
        actual = sha256_file(path)
        try:
            expected = expected_checksum(checksums, name)
        except ValueError as error:
            raise ReleaseEvidenceError(str(error)) from error
        if actual != expected:
            raise ReleaseEvidenceError(
                f"checksum mismatch for {name}: expected {expected}, got {actual}"
            )
        artifacts.append(
            {
                "path": name,
                "bytes": path.stat().st_size,
                "sha256": actual,
            }
        )

    signing_status = "verified_by_receipt" if signing_required else "not_required"
    return {
        "schema_version": 1,
        "release_tag": tag,
        "source_commit": normalized_commit,
        "release_scope": {
            "name": scope,
            "desktop_required": desktop_required,
        },
        "checksum_manifest": {
            "path": "checksums.txt",
            "sha256": sha256_file(files["checksums.txt"]),
            "entries": len(checksums),
        },
        "signature": {
            "signature_path": "checksums.txt.sig",
            "certificate_path": "checksums.txt.pem",
            "signature_sha256": sha256_file(files["checksums.txt.sig"]),
            "certificate_sha256": sha256_file(files["checksums.txt.pem"]),
            "verified_in_workflow": True,
            "oidc_issuer": "https://token.actions.githubusercontent.com",
        },
        "platform_signing": {
            "required": signing_required,
            "windows_authenticode": signing_status,
            "apple_codesign": signing_status,
            "apple_notarization": signing_status,
            "independent_published_verification_job": (
                "smoke-desktop" if desktop_required else None
            ),
            "receipts": signing_receipt_entries,
        },
        "stable_rollback": {
            "state": stable_rollback_state(tag),
            "post_publication_job": "stable-rollback-evidence",
        },
        "sbom": {
            "path": "pandora.spdx.json",
            "sha256": sha256_file(files["pandora.spdx.json"]),
        },
        "provenance": {
            "verified_in_workflow": True,
            "subjects": [
                item["path"]
                for item in artifacts
                if str(item["path"]).startswith(("pandora-", "desktop-"))
            ],
        },
        "artifacts": artifacts,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Build Pandora release evidence index")
    parser.add_argument("tag")
    parser.add_argument("--dist", type=Path, default=Path("dist"))
    parser.add_argument("--scope", choices=sorted(_SCOPES), default="full")
    parser.add_argument("--commit")
    parser.add_argument(
        "--signing-receipt",
        action="append",
        type=Path,
        default=[],
        help="hash-bound platform receipt JSON; repeat for each vendor artifact",
    )
    parser.add_argument("--output", type=Path, default=Path("dist/release-evidence.json"))
    arguments = parser.parse_args()

    try:
        evidence = build_release_evidence(
            arguments.tag,
            arguments.dist,
            scope=arguments.scope,
            commit=arguments.commit,
            signing_receipts=arguments.signing_receipt,
        )
        arguments.output.parent.mkdir(parents=True, exist_ok=True)
        arguments.output.write_text(
            json.dumps(evidence, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    except (OSError, ReleaseEvidenceError, SigningReceiptError) as error:
        print(f"error: {error}")
        return 1

    print(f"release evidence written to {arguments.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
