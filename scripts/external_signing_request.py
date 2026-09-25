"""Provider-neutral request contract for an isolated desktop signer.

The request is produced from secretless, immutable build artifacts and contains
no source, credentials, provider URLs, or executable instructions. A provider
adapter must verify the request against its own trusted release policy before
signing. This module validates shape and artifact binding only; it does not
establish provider authenticity or perform signing.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import re
import sys
from pathlib import Path
from typing import Mapping, Sequence


_SCHEMA_VERSION = 1
_REQUEST_TYPE = "pandora-desktop-signing-v1"
_RELEASE_TAG = re.compile(
    r"^v[0-9]+\.[0-9]+\.[0-9]+(?:-(?:alpha|beta|rc)\.[0-9]+)?$"
)
_TARGET = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,95}$")
_SAFE_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9 ._-]{0,127}$")
_SHA256 = re.compile(r"^[0-9a-f]{64}$")
_REQUEST_ID = re.compile(r"^sha256:[0-9a-f]{64}$")
_COMMIT = re.compile(r"^(?:[0-9a-f]{40}|[0-9a-f]{64})$")
_MAX_ARTIFACTS = 32
_MAX_ARTIFACT_BYTES = 8 * 1024 * 1024 * 1024
_MAX_REQUEST_BYTES = 256 * 1024
_PLATFORM_RULES = {
    "windows": {
        "kinds": frozenset({"exe", "msi"}),
        "checks": ("authenticode",),
    },
    "macos": {
        "kinds": frozenset({"dmg"}),
        "checks": ("codesign", "notarization"),
    },
}
_ALLOWED_REQUEST_KEYS = {
    "schema_version",
    "request_type",
    "request_id",
    "release_tag",
    "source_commit",
    "target",
    "platform",
    "required_checks",
    "policy",
    "artifacts",
}
_ALLOWED_POLICY_KEYS = {"source_execution", "rebuild", "artifact_count"}
_ALLOWED_ARTIFACT_KEYS = {"name", "kind", "sha256", "bytes"}


class ExternalSigningRequestError(ValueError):
    """Raised when an external signing request is malformed or substituted."""


def _validate_release_tag(value: object) -> str:
    if type(value) is not str or _RELEASE_TAG.fullmatch(value) is None:
        raise ExternalSigningRequestError("invalid release tag")
    return value


def _validate_commit(value: object) -> str:
    if type(value) is not str or _COMMIT.fullmatch(value) is None:
        raise ExternalSigningRequestError(
            "source_commit must be a lowercase hexadecimal Git object id"
        )
    return value


def _validate_target(value: object) -> str:
    if type(value) is not str or _TARGET.fullmatch(value) is None:
        raise ExternalSigningRequestError("invalid signing target")
    return value


def _validate_platform(value: object) -> str:
    if type(value) is not str or value not in _PLATFORM_RULES:
        raise ExternalSigningRequestError("platform must be macos or windows")
    return value


def _validate_target_platform(target: str, platform: str) -> None:
    if platform == "macos" and not target.endswith("-apple-darwin"):
        raise ExternalSigningRequestError("target does not match macOS platform")
    if platform == "windows" and not target.endswith("-pc-windows-msvc"):
        raise ExternalSigningRequestError("target does not match Windows platform")


def _validate_name(value: object) -> str:
    if type(value) is not str or _SAFE_NAME.fullmatch(value) is None:
        raise ExternalSigningRequestError(
            "artifact name must be a single safe file name"
        )
    return value


def _validate_kind(platform: str, kind: object) -> str:
    if type(kind) is not str or kind not in _PLATFORM_RULES[platform]["kinds"]:
        raise ExternalSigningRequestError("artifact kind is not valid for platform")
    return kind


def _validate_digest(value: object) -> str:
    if type(value) is not str or _SHA256.fullmatch(value) is None:
        raise ExternalSigningRequestError("artifact sha256 must be lowercase hex")
    return value


def _validate_bytes(value: object) -> int:
    if type(value) is not int or value <= 0 or value > _MAX_ARTIFACT_BYTES:
        raise ExternalSigningRequestError("artifact bytes are outside the allowed range")
    return value


def _hash_file(path: Path) -> tuple[str, int]:
    if path.is_symlink() or not path.is_file():
        raise ExternalSigningRequestError(f"artifact is not a regular file: {path.name}")
    try:
        before = path.stat()
        digest = hashlib.sha256()
        size = 0
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                size += len(chunk)
                digest.update(chunk)
        after = path.stat()
    except OSError as error:
        raise ExternalSigningRequestError(
            f"could not read artifact: {path.name}"
        ) from error
    if size <= 0 or size > _MAX_ARTIFACT_BYTES:
        raise ExternalSigningRequestError("artifact bytes are outside the allowed range")
    if size != after.st_size or before.st_mtime_ns != after.st_mtime_ns:
        raise ExternalSigningRequestError(f"artifact changed while reading: {path.name}")
    return digest.hexdigest(), size


def _canonical_bytes(value: Mapping[str, object]) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=True,
    ).encode("utf-8")


def signing_request_digest(request: Mapping[str, object]) -> str:
    """Return the stable request id, excluding the id field itself."""

    body = copy.deepcopy(dict(request))
    body.pop("request_id", None)
    return "sha256:" + hashlib.sha256(_canonical_bytes(body)).hexdigest()


def build_external_signing_request(
    *,
    release_tag: str,
    source_commit: str,
    target: str,
    platform: str,
    artifacts: Sequence[tuple[Path, str]],
) -> dict[str, object]:
    """Build a deterministic request from exact local artifact files."""

    normalized_platform = _validate_platform(platform)
    normalized_target = _validate_target(target)
    _validate_target_platform(normalized_target, normalized_platform)
    if not artifacts or len(artifacts) > _MAX_ARTIFACTS:
        raise ExternalSigningRequestError("artifact list is empty or too large")

    manifest: list[dict[str, object]] = []
    seen: set[str] = set()
    for artifact_path, kind in artifacts:
        normalized_kind = _validate_kind(normalized_platform, kind)
        name = _validate_name(artifact_path.name)
        if not name.lower().endswith(
            {"exe": ".exe", "msi": ".msi", "dmg": ".dmg"}[normalized_kind]
        ):
            raise ExternalSigningRequestError("artifact name does not match kind")
        if name in seen:
            raise ExternalSigningRequestError(f"duplicate artifact name: {name}")
        seen.add(name)
        digest, size = _hash_file(artifact_path)
        manifest.append(
            {
                "name": name,
                "kind": normalized_kind,
                "sha256": digest,
                "bytes": size,
            }
        )

    body: dict[str, object] = {
        "schema_version": _SCHEMA_VERSION,
        "request_type": _REQUEST_TYPE,
        "release_tag": _validate_release_tag(release_tag),
        "source_commit": _validate_commit(source_commit),
        "target": normalized_target,
        "platform": normalized_platform,
        "required_checks": list(_PLATFORM_RULES[normalized_platform]["checks"]),
        "policy": {
            "source_execution": "forbidden",
            "rebuild": "forbidden",
            "artifact_count": len(manifest),
        },
        "artifacts": sorted(manifest, key=lambda item: str(item["name"])),
    }
    body["request_id"] = signing_request_digest(body)
    return body


def validate_external_signing_request(
    request: object,
    *,
    artifact_root: Path | None = None,
) -> dict[str, object]:
    """Validate request shape, policy, id, and optionally local artifact bytes."""

    if not isinstance(request, dict) or set(request) != _ALLOWED_REQUEST_KEYS:
        raise ExternalSigningRequestError("request has an unsupported shape")
    if (
        type(request.get("schema_version")) is not int
        or request.get("schema_version") != _SCHEMA_VERSION
    ):
        raise ExternalSigningRequestError("unsupported request schema version")
    if request.get("request_type") != _REQUEST_TYPE:
        raise ExternalSigningRequestError("unsupported request type")

    release_tag = _validate_release_tag(request.get("release_tag"))
    source_commit = _validate_commit(request.get("source_commit"))
    target = _validate_target(request.get("target"))
    platform = _validate_platform(request.get("platform"))
    _validate_target_platform(target, platform)
    expected_checks = list(_PLATFORM_RULES[platform]["checks"])
    if request.get("required_checks") != expected_checks:
        raise ExternalSigningRequestError("required checks do not match platform policy")

    policy = request.get("policy")
    if not isinstance(policy, dict) or set(policy) != _ALLOWED_POLICY_KEYS:
        raise ExternalSigningRequestError("request policy has an unsupported shape")
    if policy.get("source_execution") != "forbidden":
        raise ExternalSigningRequestError("source execution must be forbidden")
    if policy.get("rebuild") != "forbidden":
        raise ExternalSigningRequestError("rebuild must be forbidden")

    artifacts = request.get("artifacts")
    if not isinstance(artifacts, list) or not artifacts or len(artifacts) > _MAX_ARTIFACTS:
        raise ExternalSigningRequestError("artifact list is empty or too large")
    if (
        type(policy.get("artifact_count")) is not int
        or policy.get("artifact_count") != len(artifacts)
    ):
        raise ExternalSigningRequestError("artifact_count does not match artifact list")

    normalized: list[dict[str, object]] = []
    seen: set[str] = set()
    for item in artifacts:
        if not isinstance(item, dict) or set(item) != _ALLOWED_ARTIFACT_KEYS:
            raise ExternalSigningRequestError("artifact entry has an unsupported shape")
        name = _validate_name(item.get("name"))
        if name in seen:
            raise ExternalSigningRequestError(f"duplicate artifact name: {name}")
        seen.add(name)
        kind = _validate_kind(platform, item.get("kind"))
        suffix = {"exe": ".exe", "msi": ".msi", "dmg": ".dmg"}[kind]
        if not name.lower().endswith(suffix):
            raise ExternalSigningRequestError("artifact name does not match kind")
        digest = _validate_digest(item.get("sha256"))
        size = _validate_bytes(item.get("bytes"))
        if artifact_root is not None:
            if artifact_root.is_symlink() or not artifact_root.is_dir():
                raise ExternalSigningRequestError("artifact root is not a regular directory")
            actual_digest, actual_size = _hash_file(artifact_root / name)
            if actual_digest != digest or actual_size != size:
                raise ExternalSigningRequestError(f"artifact binding mismatch: {name}")
        normalized.append(
            {"name": name, "kind": kind, "sha256": digest, "bytes": size}
        )

    normalized.sort(key=lambda item: str(item["name"]))
    expected = copy.deepcopy(request)
    expected["release_tag"] = release_tag
    expected["source_commit"] = source_commit
    expected["target"] = target
    expected["platform"] = platform
    expected["required_checks"] = expected_checks
    expected["artifacts"] = normalized
    expected_id = signing_request_digest(expected)
    if request.get("request_id") != expected_id or _REQUEST_ID.fullmatch(str(request.get("request_id"))) is None:
        raise ExternalSigningRequestError("request_id does not match request contents")
    return copy.deepcopy(request)


def _reject_duplicate_json_keys(pairs: list[tuple[str, object]]) -> dict[str, object]:
    document: dict[str, object] = {}
    for key, value in pairs:
        if key in document:
            raise ExternalSigningRequestError(f"duplicate JSON key: {key}")
        document[key] = value
    return document


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate a provider-neutral external signing request"
    )
    parser.add_argument("request", type=Path)
    parser.add_argument("--artifacts-root", type=Path)
    arguments = parser.parse_args()
    try:
        if arguments.request.is_symlink() or not arguments.request.is_file():
            raise ExternalSigningRequestError("request is not a regular file")
        with arguments.request.open("rb") as handle:
            raw = handle.read(_MAX_REQUEST_BYTES + 1)
        if len(raw) > _MAX_REQUEST_BYTES:
            raise ExternalSigningRequestError("request exceeds the size limit")
        document = json.loads(
            raw.decode("utf-8"),
            object_pairs_hook=_reject_duplicate_json_keys,
        )
        request = validate_external_signing_request(
            document,
            artifact_root=arguments.artifacts_root,
        )
    except (OSError, UnicodeDecodeError, json.JSONDecodeError, ExternalSigningRequestError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"external signing request validated: {request['request_id']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
