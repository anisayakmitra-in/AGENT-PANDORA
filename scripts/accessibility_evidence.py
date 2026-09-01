from __future__ import annotations

import argparse
import hashlib
import json
import re
from datetime import datetime
from pathlib import Path
from typing import Any


SHA256 = re.compile(r"^[0-9a-f]{64}$")
COMMIT_SHA = re.compile(r"^[0-9a-f]{40}$")
SEMVER = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-(?:alpha|beta|rc)\.[1-9][0-9]*)?$"
)
PLATFORMS = {
    "windows-x64": {"architecture": "x86_64", "assistive_technology": "NVDA"},
    "linux-x64": {"architecture": "x86_64", "assistive_technology": "Orca"},
    "macos-x64": {"architecture": "x86_64", "assistive_technology": "VoiceOver"},
    "macos-arm64": {"architecture": "arm64", "assistive_technology": "VoiceOver"},
}
CHECKS = (
    "landmarks",
    "controls",
    "forms",
    "status_changes",
    "dialogs",
    "keyboard_order",
    "visible_focus",
    "scaling",
    "high_contrast",
    "forced_colors",
    "reduced_motion",
    "reduced_transparency",
    "minimum_window",
    "install",
    "start",
    "update",
    "rollback",
    "uninstall",
)
REQUIRED_EVIDENCE_KINDS = {
    "screenshot",
    "screen-reader-notes",
    "lifecycle-log",
}


class AccessibilityEvidenceError(ValueError):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_object(value: Any, description: str, keys: set[str]) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise AccessibilityEvidenceError(f"{description} must be an object")
    actual = set(value)
    if actual != keys:
        missing = sorted(keys - actual)
        unexpected = sorted(actual - keys)
        raise AccessibilityEvidenceError(
            f"{description} has an unexpected shape; missing={missing}, unexpected={unexpected}"
        )
    return value


def require_string(value: Any, description: str, maximum: int = 200) -> str:
    if not isinstance(value, str) or not value.strip():
        raise AccessibilityEvidenceError(f"{description} must be a non-empty string")
    normalized = value.strip()
    if len(normalized) > maximum or any(ord(character) < 32 for character in normalized):
        raise AccessibilityEvidenceError(f"{description} is invalid")
    return normalized


def require_bool(value: Any, description: str) -> bool:
    if not isinstance(value, bool):
        raise AccessibilityEvidenceError(f"{description} must be a boolean")
    return value


def safe_asset(evidence_root: Path, relative_value: Any, description: str) -> Path:
    relative_text = require_string(relative_value, description, 300)
    if "\\" in relative_text:
        raise AccessibilityEvidenceError(f"{description} must use portable forward slashes")
    relative = Path(relative_text)
    if relative.is_absolute() or ".." in relative.parts or relative.name in {"", ".", ".."}:
        raise AccessibilityEvidenceError(f"{description} escapes the evidence directory")
    candidate = evidence_root.joinpath(*relative.parts)
    try:
        canonical_root = evidence_root.resolve(strict=True)
        canonical = candidate.resolve(strict=True)
        canonical.relative_to(canonical_root)
    except (FileNotFoundError, OSError, ValueError) as error:
        raise AccessibilityEvidenceError(
            f"{description} is missing or escapes the evidence directory: {relative_text}"
        ) from error
    current = candidate
    while current != evidence_root:
        if current.is_symlink():
            raise AccessibilityEvidenceError(f"{description} must not contain symlinks")
        current = current.parent
    if canonical.is_symlink() or not canonical.is_file() or canonical.stat().st_size == 0:
        raise AccessibilityEvidenceError(f"{description} must be a non-empty regular file")
    return canonical


def parse_timestamp(value: Any, description: str) -> str:
    timestamp = require_string(value, description)
    if not timestamp.endswith("Z"):
        raise AccessibilityEvidenceError(f"{description} must be an explicit UTC timestamp")
    try:
        datetime.fromisoformat(timestamp[:-1] + "+00:00")
    except ValueError as error:
        raise AccessibilityEvidenceError(f"{description} is invalid") from error
    return timestamp


def validate_manifest(
    manifest_path: Path,
    evidence_root: Path,
    expected_platform: str,
    expected_commit: str,
) -> dict[str, Any]:
    if manifest_path.is_symlink() or not manifest_path.is_file():
        raise AccessibilityEvidenceError(
            f"{manifest_path.name} must be a regular manifest file and not a symlink"
        )
    if manifest_path.stat().st_size > 1024 * 1024:
        raise AccessibilityEvidenceError(f"{manifest_path.name} exceeds the manifest size limit")
    try:
        raw = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise AccessibilityEvidenceError(f"{manifest_path.name} is invalid JSON") from error

    record = require_object(
        raw,
        manifest_path.name,
        {
            "schema_version",
            "commit_sha",
            "artifact",
            "platform",
            "assistive_technology",
            "session",
            "release_identity",
            "checks",
            "evidence",
            "findings",
        },
    )
    if record["schema_version"] != 1:
        raise AccessibilityEvidenceError(f"{manifest_path.name} has an unsupported schema version")
    commit_sha = require_string(record["commit_sha"], f"{manifest_path.name}.commit_sha")
    if COMMIT_SHA.fullmatch(commit_sha) is None or commit_sha != expected_commit:
        raise AccessibilityEvidenceError(
            f"{manifest_path.name} is not bound to exact commit {expected_commit}"
        )

    artifact = require_object(
        record["artifact"],
        f"{manifest_path.name}.artifact",
        {"name", "sha256", "signed", "notarized"},
    )
    artifact_name = require_string(artifact["name"], f"{manifest_path.name}.artifact.name")
    if Path(artifact_name).name != artifact_name or "/" in artifact_name or "\\" in artifact_name:
        raise AccessibilityEvidenceError(f"{manifest_path.name}.artifact.name must be a filename")
    artifact_digest = require_string(
        artifact["sha256"], f"{manifest_path.name}.artifact.sha256"
    )
    if SHA256.fullmatch(artifact_digest) is None:
        raise AccessibilityEvidenceError(f"{manifest_path.name}.artifact.sha256 is invalid")
    require_bool(artifact["signed"], f"{manifest_path.name}.artifact.signed")
    require_bool(artifact["notarized"], f"{manifest_path.name}.artifact.notarized")

    platform = require_object(
        record["platform"],
        f"{manifest_path.name}.platform",
        {"id", "os_name", "os_version", "architecture", "scales_tested", "minimum_window"},
    )
    platform_id = require_string(platform["id"], f"{manifest_path.name}.platform.id")
    if platform_id != expected_platform:
        raise AccessibilityEvidenceError(
            f"{manifest_path.name} declares {platform_id}, expected {expected_platform}"
        )
    require_string(platform["os_name"], f"{manifest_path.name}.platform.os_name")
    require_string(platform["os_version"], f"{manifest_path.name}.platform.os_version")
    if platform["architecture"] != PLATFORMS[expected_platform]["architecture"]:
        raise AccessibilityEvidenceError(f"{manifest_path.name} has the wrong architecture")
    if platform["scales_tested"] != [100, 150, 200]:
        raise AccessibilityEvidenceError(
            f"{manifest_path.name} must record exact 100%, 150%, and 200% scaling"
        )
    minimum_window = require_object(
        platform["minimum_window"],
        f"{manifest_path.name}.platform.minimum_window",
        {"width", "height"},
    )
    if minimum_window != {"width": 1080, "height": 720}:
        raise AccessibilityEvidenceError(
            f"{manifest_path.name} must exercise the supported 1080x720 minimum window"
        )

    assistive = require_object(
        record["assistive_technology"],
        f"{manifest_path.name}.assistive_technology",
        {"name", "version"},
    )
    expected_assistive = PLATFORMS[expected_platform]["assistive_technology"]
    if assistive["name"] != expected_assistive:
        raise AccessibilityEvidenceError(
            f"{manifest_path.name} must use {expected_assistive}"
        )
    require_string(
        assistive["version"], f"{manifest_path.name}.assistive_technology.version"
    )

    session = require_object(
        record["session"],
        f"{manifest_path.name}.session",
        {"tested_at", "tester", "packaged_app", "clean_machine"},
    )
    parse_timestamp(session["tested_at"], f"{manifest_path.name}.session.tested_at")
    require_string(session["tester"], f"{manifest_path.name}.session.tester")
    if require_bool(session["packaged_app"], f"{manifest_path.name}.session.packaged_app") is not True:
        raise AccessibilityEvidenceError(f"{manifest_path.name} must test the packaged app")
    if require_bool(session["clean_machine"], f"{manifest_path.name}.session.clean_machine") is not True:
        raise AccessibilityEvidenceError(f"{manifest_path.name} must test a clean machine")

    release_identity = require_object(
        record["release_identity"],
        f"{manifest_path.name}.release_identity",
        {"desktop_version", "cli_version"},
    )
    desktop_version = require_string(
        release_identity["desktop_version"],
        f"{manifest_path.name}.release_identity.desktop_version",
    )
    cli_version = require_string(
        release_identity["cli_version"],
        f"{manifest_path.name}.release_identity.cli_version",
    )
    if SEMVER.fullmatch(desktop_version) is None or cli_version != desktop_version:
        raise AccessibilityEvidenceError(
            f"{manifest_path.name} desktop and CLI release identities must be the same version"
        )

    checks = require_object(
        record["checks"], f"{manifest_path.name}.checks", set(CHECKS)
    )
    failed_checks = [name for name in CHECKS if checks[name] != "pass"]
    if failed_checks:
        raise AccessibilityEvidenceError(
            f"{manifest_path.name} has incomplete or failed checks: {failed_checks}"
        )

    evidence = record["evidence"]
    if not isinstance(evidence, list) or not evidence:
        raise AccessibilityEvidenceError(f"{manifest_path.name}.evidence must be a non-empty list")
    evidence_kinds: set[str] = set()
    evidence_paths: set[str] = set()
    validated_evidence: list[dict[str, str]] = []
    for index, item in enumerate(evidence):
        entry = require_object(
            item,
            f"{manifest_path.name}.evidence[{index}]",
            {"path", "sha256", "kind"},
        )
        relative_path = require_string(
            entry["path"], f"{manifest_path.name}.evidence[{index}].path", 300
        )
        if relative_path in evidence_paths:
            raise AccessibilityEvidenceError(
                f"{manifest_path.name} repeats evidence path {relative_path}"
            )
        evidence_paths.add(relative_path)
        kind = require_string(
            entry["kind"], f"{manifest_path.name}.evidence[{index}].kind"
        )
        if kind not in REQUIRED_EVIDENCE_KINDS:
            raise AccessibilityEvidenceError(
                f"{manifest_path.name} has unsupported evidence kind {kind}"
            )
        evidence_kinds.add(kind)
        expected_digest = require_string(
            entry["sha256"], f"{manifest_path.name}.evidence[{index}].sha256"
        )
        if SHA256.fullmatch(expected_digest) is None:
            raise AccessibilityEvidenceError(
                f"{manifest_path.name} has an invalid evidence checksum"
            )
        asset = safe_asset(
            evidence_root,
            relative_path,
            f"{manifest_path.name}.evidence[{index}].path",
        )
        actual_digest = sha256_file(asset)
        if actual_digest != expected_digest:
            raise AccessibilityEvidenceError(
                f"{manifest_path.name} evidence checksum mismatch for {relative_path}"
            )
        validated_evidence.append(
            {"path": relative_path, "kind": kind, "sha256": actual_digest}
        )
    if not REQUIRED_EVIDENCE_KINDS.issubset(evidence_kinds):
        missing_kinds = sorted(REQUIRED_EVIDENCE_KINDS - evidence_kinds)
        raise AccessibilityEvidenceError(
            f"{manifest_path.name} is missing evidence kinds: {missing_kinds}"
        )

    findings = record["findings"]
    if not isinstance(findings, list):
        raise AccessibilityEvidenceError(f"{manifest_path.name}.findings must be a list")
    unresolved_critical = 0
    for index, item in enumerate(findings):
        finding = require_object(
            item,
            f"{manifest_path.name}.findings[{index}]",
            {"severity", "status", "description", "issue"},
        )
        if finding["severity"] not in {"critical", "high", "medium", "low"}:
            raise AccessibilityEvidenceError(
                f"{manifest_path.name}.findings[{index}].severity is invalid"
            )
        if finding["status"] not in {"resolved", "accepted"}:
            raise AccessibilityEvidenceError(
                f"{manifest_path.name}.findings[{index}].status is invalid"
            )
        require_string(
            finding["description"],
            f"{manifest_path.name}.findings[{index}].description",
            1000,
        )
        require_string(
            finding["issue"], f"{manifest_path.name}.findings[{index}].issue", 300
        )
        if finding["severity"] == "critical" and finding["status"] != "resolved":
            unresolved_critical += 1
    if unresolved_critical:
        raise AccessibilityEvidenceError(
            f"{manifest_path.name} has unresolved critical accessibility findings"
        )

    return {
        "platform": expected_platform,
        "manifest": manifest_path.name,
        "manifest_sha256": sha256_file(manifest_path),
        "artifact": {"name": artifact_name, "sha256": artifact_digest},
        "assistive_technology": {
            "name": expected_assistive,
            "version": assistive["version"],
        },
        "tested_at": session["tested_at"],
        "release_identity": desktop_version,
        "evidence": validated_evidence,
        "findings": len(findings),
        "unresolved_critical_findings": 0,
    }


def validate_evidence_set(
    directory: Path, expected_commit: str, root: Path | None = None
) -> dict[str, Any]:
    if COMMIT_SHA.fullmatch(expected_commit) is None:
        raise AccessibilityEvidenceError("expected commit must be a lowercase 40-character SHA")
    repository_root = (root or Path.cwd()).resolve(strict=True)
    requested = directory if directory.is_absolute() else repository_root / directory
    try:
        evidence_root = requested.resolve(strict=True)
        evidence_root.relative_to(repository_root)
    except (FileNotFoundError, OSError, ValueError) as error:
        raise AccessibilityEvidenceError(
            "evidence directory must exist inside the repository root"
        ) from error
    if requested.is_symlink() or not evidence_root.is_dir():
        raise AccessibilityEvidenceError("evidence directory must be a directory and not a symlink")

    expected_manifests = {f"{platform}.json" for platform in PLATFORMS}
    actual_manifests = {path.name for path in evidence_root.glob("*.json")}
    if actual_manifests != expected_manifests:
        raise AccessibilityEvidenceError(
            "evidence directory must contain exactly one root manifest for every advertised "
            f"platform; missing={sorted(expected_manifests - actual_manifests)}, "
            f"unexpected={sorted(actual_manifests - expected_manifests)}"
        )

    records = [
        validate_manifest(
            evidence_root / f"{platform}.json",
            evidence_root,
            platform,
            expected_commit,
        )
        for platform in PLATFORMS
    ]
    release_identities = {record["release_identity"] for record in records}
    if len(release_identities) != 1:
        raise AccessibilityEvidenceError(
            "all native accessibility records must use the same desktop and CLI release identity"
        )
    return {
        "schema_version": 1,
        "complete": True,
        "commit_sha": expected_commit,
        "release_identity": next(iter(release_identities)),
        "platforms": records,
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Validate retained native accessibility and clean-machine evidence"
    )
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--output", type=Path)
    arguments = parser.parse_args()
    try:
        index = validate_evidence_set(
            arguments.directory,
            arguments.commit,
            arguments.root,
        )
        rendered = json.dumps(index, indent=2, sort_keys=True) + "\n"
        if arguments.output:
            arguments.output.parent.mkdir(parents=True, exist_ok=True)
            arguments.output.write_text(rendered, encoding="utf-8")
        else:
            print(rendered, end="")
    except (AccessibilityEvidenceError, OSError) as error:
        print(f"error: {error}")
        return 1
    print("native accessibility evidence is complete and exact")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
