"""Validate hash-bound receipts from isolated platform-signing jobs.

A receipt records the result of vendor-tool verification without containing
credentials or private key material. Callers must still run the named verifier
before constructing a successful receipt; this module validates and binds the
resulting evidence, and never performs signing itself.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Mapping


_RELEASE_TAG = re.compile(
    r"^v[0-9]+\.[0-9]+\.[0-9]+(?:-(?:alpha|beta|rc)\.[0-9]+)?$"
)
_SHA256 = re.compile(r"^[0-9a-f]{64}$")
_COMMIT = re.compile(r"^(?:[0-9a-f]{40}|[0-9a-f]{64})$")
_RECEIPT_TYPE = "platform-signing-v1"
_ALLOWED_RECEIPT_KEYS = {
    "schema_version",
    "receipt_type",
    "release_tag",
    "commit",
    "platform",
    "artifact",
    "checks",
}
_ALLOWED_ARTIFACT_KEYS = {"path", "sha256", "bytes"}
_ALLOWED_CHECK_KEYS = {"name", "required", "verified", "verifier"}
_CHECKS_BY_PLATFORM = {
    "windows": ("authenticode",),
    "macos": ("codesign", "notarization"),
}
_VERIFIERS_BY_CHECK = {
    "authenticode": "signtool-verify-v1",
    "codesign": "codesign-verify-v1",
    "notarization": "stapler-validate-v1",
}
_MAX_RECEIPT_BYTES = 64 * 1024


class SigningReceiptError(ValueError):
    pass


def _validate_tag(tag: str) -> None:
    if type(tag) is not str or _RELEASE_TAG.fullmatch(tag) is None:
        raise SigningReceiptError(f"invalid release tag: {tag}")


def _validate_commit(commit: str) -> None:
    if type(commit) is not str or _COMMIT.fullmatch(commit) is None:
        raise SigningReceiptError("commit must be a lowercase hexadecimal Git object id")


def _validate_artifact_name(name: object) -> str:
    if (
        type(name) is not str
        or not name
        or name in {".", ".."}
        or "/" in name
        or "\\" in name
    ):
        raise SigningReceiptError("receipt artifact path must be a single file name")
    return name


def _validate_platform(platform: object) -> str:
    if type(platform) is not str or platform not in _CHECKS_BY_PLATFORM:
        raise SigningReceiptError("receipt platform must be windows or macos")
    return platform


def _file_digest_and_size(path: Path) -> tuple[str, int]:
    if path.is_symlink() or not path.is_file():
        raise SigningReceiptError(f"receipt artifact is not a regular file: {path}")
    digest = hashlib.sha256()
    size = 0
    try:
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                size += len(chunk)
                digest.update(chunk)
    except OSError as error:
        raise SigningReceiptError(f"could not read receipt artifact: {path}") from error
    return digest.hexdigest(), size


def build_platform_signing_receipt(
    *,
    release_tag: str,
    commit: str,
    platform: str,
    artifact_path: Path,
    verifications: Mapping[str, bool],
) -> dict[str, object]:
    _validate_tag(release_tag)
    _validate_commit(commit)
    normalized_platform = _validate_platform(platform)
    artifact_name = _validate_artifact_name(artifact_path.name)
    expected_checks = _CHECKS_BY_PLATFORM[normalized_platform]
    if not isinstance(verifications, Mapping):
        raise SigningReceiptError("verification results must be a mapping")
    if set(verifications) != set(expected_checks):
        raise SigningReceiptError(
            f"{normalized_platform} receipt requires checks: "
            + ", ".join(expected_checks)
        )

    digest, size = _file_digest_and_size(artifact_path)
    checks: list[dict[str, object]] = []
    for name in expected_checks:
        verified = verifications[name]
        if type(verified) is not bool:
            raise SigningReceiptError(f"verification result for {name} must be boolean")
        checks.append(
            {
                "name": name,
                "required": True,
                "verified": verified,
                "verifier": _VERIFIERS_BY_CHECK[name],
            }
        )
    return {
        "schema_version": 1,
        "receipt_type": _RECEIPT_TYPE,
        "release_tag": release_tag,
        "commit": commit,
        "platform": normalized_platform,
        "artifact": {
            "path": artifact_name,
            "sha256": digest,
            "bytes": size,
        },
        "checks": checks,
    }


def receipt_digest(receipt: Mapping[str, object]) -> str:
    canonical = json.dumps(
        receipt,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=True,
    ).encode("utf-8")
    return hashlib.sha256(canonical).hexdigest()


def read_platform_signing_receipt(path: Path) -> tuple[object, str]:
    try:
        if path.is_symlink() or not path.is_file():
            raise SigningReceiptError(f"signing receipt is not a regular file: {path}")
        raw = path.read_bytes()
    except SigningReceiptError:
        raise
    except OSError as error:
        raise SigningReceiptError(f"could not read signing receipt: {path}") from error
    if len(raw) > _MAX_RECEIPT_BYTES:
        raise SigningReceiptError("signing receipt exceeds the size limit")
    try:
        document = json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=_reject_duplicate_json_keys,
        )
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise SigningReceiptError(f"could not decode signing receipt: {path}") from error
    return document, hashlib.sha256(raw).hexdigest()


def load_platform_signing_receipt(path: Path) -> object:
    document, _ = read_platform_signing_receipt(path)
    return document


def validate_platform_signing_receipt(
    receipt: object,
    artifact_path: Path,
    *,
    expected_tag: str,
    expected_commit: str,
    require_verified: bool = True,
) -> dict[str, object]:
    _validate_tag(expected_tag)
    _validate_commit(expected_commit)
    if not isinstance(receipt, dict) or set(receipt) != _ALLOWED_RECEIPT_KEYS:
        raise SigningReceiptError("receipt has an unsupported shape")
    if type(receipt.get("schema_version")) is not int or receipt.get("schema_version") != 1:
        raise SigningReceiptError("receipt schema_version must be 1")
    if receipt.get("receipt_type") != _RECEIPT_TYPE:
        raise SigningReceiptError("unsupported platform signing receipt type")
    if receipt.get("release_tag") != expected_tag:
        raise SigningReceiptError("receipt release tag does not match expected tag")
    if receipt.get("commit") != expected_commit:
        raise SigningReceiptError("receipt commit does not match expected commit")

    platform = _validate_platform(receipt.get("platform"))
    artifact = receipt.get("artifact")
    if not isinstance(artifact, dict) or set(artifact) != _ALLOWED_ARTIFACT_KEYS:
        raise SigningReceiptError("receipt artifact has an unsupported shape")
    artifact_name = _validate_artifact_name(artifact.get("path"))
    if artifact_name != artifact_path.name:
        raise SigningReceiptError("receipt artifact name does not match the artifact")
    digest = artifact.get("sha256")
    size = artifact.get("bytes")
    if type(digest) is not str or _SHA256.fullmatch(digest) is None:
        raise SigningReceiptError("receipt artifact digest is invalid")
    if type(size) is not int or size < 0:
        raise SigningReceiptError("receipt artifact byte count is invalid")

    actual_digest, actual_size = _file_digest_and_size(artifact_path)
    if actual_digest != digest:
        raise SigningReceiptError(
            f"receipt artifact digest mismatch: expected {digest}, got {actual_digest}"
        )
    if actual_size != size:
        raise SigningReceiptError(
            f"receipt artifact byte count mismatch: expected {size}, got {actual_size}"
        )

    expected_checks = _CHECKS_BY_PLATFORM[platform]
    checks = receipt.get("checks")
    if not isinstance(checks, list) or len(checks) != len(expected_checks):
        raise SigningReceiptError("receipt has an incomplete platform check list")
    for index, expected_name in enumerate(expected_checks):
        check = checks[index]
        if not isinstance(check, dict) or set(check) != _ALLOWED_CHECK_KEYS:
            raise SigningReceiptError("receipt check has an unsupported shape")
        if check.get("name") != expected_name:
            raise SigningReceiptError("receipt checks are out of order or unexpected")
        if check.get("required") is not True:
            raise SigningReceiptError("receipt checks must be required")
        if type(check.get("verified")) is not bool:
            raise SigningReceiptError("receipt verification result must be boolean")
        if check.get("verifier") != _VERIFIERS_BY_CHECK[expected_name]:
            raise SigningReceiptError("receipt verifier does not match the platform check")

    if require_verified and not all(check["verified"] is True for check in checks):
        raise SigningReceiptError("platform signing receipt is not verified")
    return copy.deepcopy(receipt)


def _reject_duplicate_json_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    document: dict[str, object] = {}
    for key, value in pairs:
        if key in document:
            raise SigningReceiptError(f"duplicate JSON key: {key}")
        document[key] = value
    return document


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate a hash-bound platform signing receipt"
    )
    parser.add_argument("receipt", type=Path)
    parser.add_argument("--artifact", type=Path, required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument(
        "--allow-failure",
        action="store_true",
        help="validate binding and shape while permitting a failed verification result",
    )
    arguments = parser.parse_args()

    try:
        receipt = load_platform_signing_receipt(arguments.receipt)
        validate_platform_signing_receipt(
            receipt,
            arguments.artifact,
            expected_tag=arguments.tag,
            expected_commit=arguments.commit,
            require_verified=not arguments.allow_failure,
        )
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, SigningReceiptError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    result_label = (
        "platform signing receipt validated with failure result"
        if arguments.allow_failure
        else "platform signing receipt verified"
    )
    print(f"{result_label}: {receipt_digest(receipt)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
