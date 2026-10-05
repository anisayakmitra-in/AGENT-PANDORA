from __future__ import annotations

import argparse
import hashlib
import json
import re
from pathlib import Path

try:
    from .installer_contract import expected_checksum, parse_checksums
except ImportError:
    from installer_contract import expected_checksum, parse_checksums


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

# Schema 2 drops the platform_signing block: Pandora is a terminal CLI whose
# release binaries are not OS-signed. Schema 1 evidence remains readable, so
# evidence produced before this change still validates.
SCHEMA_VERSION = 2
_SUPPORTED_EVIDENCE_SCHEMA_VERSIONS = (1, 2)


class ReleaseEvidenceError(ValueError):
    pass


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


def _validate_commit(commit: str | None) -> str | None:
    if commit is None:
        return None
    if _GIT_COMMIT.fullmatch(commit) is None:
        raise ReleaseEvidenceError("release evidence commit is invalid")
    return commit


def build_release_evidence(
    tag: str,
    dist: Path,
    *,
    scope: str = "full",
    commit: str | None = None,
) -> dict[str, object]:
    if _RELEASE_TAG.fullmatch(tag) is None:
        raise ReleaseEvidenceError(f"invalid release tag: {tag}")
    if type(scope) is not str or scope not in _SCOPES:
        raise ReleaseEvidenceError(f"unsupported release scope: {scope}")
    normalized_commit = _validate_commit(commit)
    desktop_required = scope == "full"

    files = _release_files(dist)
    _require_files(files, desktop_required=desktop_required)
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

    return {
        "schema_version": SCHEMA_VERSION,
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
            "status": "not_applicable",
            "reason": (
                "Pandora is a terminal CLI distributed through channels that do not "
                "require an OS signature. Release binaries are not Authenticode- or "
                "codesign-signed. Integrity is established by checksums.txt, its "
                "cosign signature, and build attestations."
            ),
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
    parser.add_argument("--output", type=Path, default=Path("dist/release-evidence.json"))
    arguments = parser.parse_args()

    try:
        evidence = build_release_evidence(
            arguments.tag,
            arguments.dist,
            scope=arguments.scope,
            commit=arguments.commit,
        )
        arguments.output.parent.mkdir(parents=True, exist_ok=True)
        arguments.output.write_text(
            json.dumps(evidence, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    except (OSError, ReleaseEvidenceError) as error:
        print(f"error: {error}")
        return 1

    print(f"release evidence written to {arguments.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
