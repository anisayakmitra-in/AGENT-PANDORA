from __future__ import annotations

import argparse
import hashlib
import json
import re
import stat
import subprocess
import sys
from pathlib import Path, PurePosixPath
from typing import Any


SHA256 = re.compile(r"^sha256:[0-9a-f]{64}$")
COMMIT = re.compile(r"^[0-9a-f]{40}$")
CHANNELS = {"beta", "release-candidate", "stable"}
EVIDENCE_FILES = (
    "package-validation.jsonl",
    "evaluation.json",
    "pipeline.json",
)
MAX_EVIDENCE_FILE_BYTES = 16 * 1024 * 1024
# Match the runtime's stored-package artifact ceiling.
MAX_PROMOTION_ARTIFACT_BYTES = 16 * 1024 * 1024


class EvidenceError(ValueError):
    pass


def canonical_json(value: Any) -> bytes:
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode("utf-8")


def digest_bytes(value: bytes) -> str:
    return f"sha256:{hashlib.sha256(value).hexdigest()}"


def _require_identity(commit: str, channel: str) -> None:
    if COMMIT.fullmatch(commit) is None:
        raise EvidenceError("commit must be a lowercase 40-character Git SHA")
    if channel not in CHANNELS:
        raise EvidenceError("channel is not supported")


def _require_checked_out_commit(repository_root: Path, expected_commit: str) -> None:
    repository_root = repository_root.resolve(strict=True)
    result = subprocess.run(
        ["git", "rev-parse", "--verify", "HEAD^{commit}"],
        cwd=repository_root,
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0 or result.stdout.strip() != expected_commit:
        raise EvidenceError("checked-out source commit does not match the evidence identity")


def _regular_file(root: Path, relative: str, max_bytes: int = MAX_EVIDENCE_FILE_BYTES) -> Path:
    candidate = root.joinpath(*PurePosixPath(relative).parts)
    try:
        resolved_root = root.resolve(strict=True)
        current = resolved_root
        for part in PurePosixPath(relative).parts:
            current = current / part
            metadata = current.lstat()
            if stat.S_ISLNK(metadata.st_mode):
                raise EvidenceError(f"evidence path contains a symlink: {relative}")
        resolved = candidate.resolve(strict=True)
    except FileNotFoundError as error:
        raise EvidenceError(f"required evidence file is missing: {relative}") from error
    try:
        resolved.relative_to(resolved_root)
    except ValueError as error:
        raise EvidenceError(f"evidence path escapes its root: {relative}") from error
    if not resolved.is_file():
        raise EvidenceError(f"evidence path is not a regular file: {relative}")
    if resolved.stat().st_size > max_bytes:
        raise EvidenceError(f"evidence file exceeds the size limit: {relative}")
    return resolved


def _tracked_blob(repository_root: Path, relative: str) -> bytes:
    tracked = subprocess.run(
        ["git", "ls-files", "-z", "--error-unmatch", "--", relative],
        cwd=repository_root,
        check=False,
        capture_output=True,
    )
    if tracked.returncode != 0 or tracked.stdout != relative.encode("utf-8") + b"\0":
        raise EvidenceError(f"artifact package file is not tracked at the checked-out commit: {relative}")
    committed = subprocess.run(
        ["git", "show", f"HEAD:{relative}"],
        cwd=repository_root,
        check=False,
        capture_output=True,
    )
    if committed.returncode != 0:
        raise EvidenceError(f"could not read artifact package file from HEAD: {relative}")
    return committed.stdout


def evidence_payload(directory: Path, commit: str, channel: str) -> dict[str, Any]:
    _require_identity(commit, channel)
    files = []
    for relative in EVIDENCE_FILES:
        file_path = _regular_file(directory, relative)
        content = file_path.read_bytes()
        files.append({"path": relative, "size": len(content), "sha256": hashlib.sha256(content).hexdigest()})
    _validate_evidence_content(directory, commit, channel)
    return {
        "schema_version": 1,
        "commit": commit,
        "channel": channel,
        "files": files,
    }


def _validate_evidence_content(directory: Path, commit: str, channel: str) -> None:
    package_lines = _regular_file(directory, "package-validation.jsonl").read_text(encoding="utf-8").splitlines()
    if not package_lines:
        raise EvidenceError("package validation report is empty")
    for number, line in enumerate(package_lines, start=1):
        try:
            package_result = json.loads(line)
        except json.JSONDecodeError as error:
            raise EvidenceError(f"package validation line {number} is invalid JSON") from error
        if (
            not isinstance(package_result, dict)
            or package_result.get("command") != "package validate"
            or package_result.get("valid") is not True
            or package_result.get("persisted") is not False
        ):
            raise EvidenceError(f"package validation line {number} is not a successful non-persisting validation")

    try:
        evaluation = json.loads(_regular_file(directory, "evaluation.json").read_text(encoding="utf-8"))
        pipeline = json.loads(_regular_file(directory, "pipeline.json").read_text(encoding="utf-8"))
    except json.JSONDecodeError as error:
        raise EvidenceError("evaluation or pipeline evidence is invalid JSON") from error
    if (
        not isinstance(evaluation, dict)
        or evaluation.get("command") != "evaluation golden"
        or type(evaluation.get("total")) is not int
        or type(evaluation.get("passed")) is not int
        or type(evaluation.get("failed")) is not int
        or evaluation.get("total", 0) <= 0
        or evaluation.get("failed") != 0
        or evaluation.get("passed") != evaluation.get("total")
    ):
        raise EvidenceError("evaluation evidence is not a complete passing golden-set report")
    expected_pipeline = {
        "schema_version": 1,
        "commit": commit,
        "channel": channel,
        "package_admission_performed": False,
        "signed_distribution_boundary_tested": True,
        "canary_stops_before_activation_tested": True,
        "artifact_activation_performed": False,
        "release_tag_created": False,
        "authority": "evidence_only",
    }
    if not isinstance(pipeline, dict):
        raise EvidenceError("pipeline boundary evidence is not a JSON object")
    if type(pipeline.get("schema_version")) is not int or any(
        type(pipeline.get(key)) is not bool
        for key in (
            "package_admission_performed",
            "signed_distribution_boundary_tested",
            "canary_stops_before_activation_tested",
            "artifact_activation_performed",
            "release_tag_created",
        )
    ):
        raise EvidenceError("pipeline boundary evidence has invalid field types")
    if pipeline != expected_pipeline:
        raise EvidenceError("pipeline boundary evidence does not match the exact commit/channel or safe boundary")


def create_manifest(
    directory: Path,
    commit: str,
    channel: str,
    repository_root: Path,
) -> dict[str, Any]:
    _require_checked_out_commit(repository_root, commit)
    payload = evidence_payload(directory, commit, channel)
    manifest = {**payload, "digest": digest_bytes(canonical_json(payload))}
    output = _regular_file_parent(directory, "evidence-manifest.json")
    output.write_text(json.dumps(manifest, ensure_ascii=False, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    return manifest


def _regular_file_parent(root: Path, relative: str) -> Path:
    pure = PurePosixPath(relative)
    if pure.is_absolute() or not pure.parts or any(part in ("", ".", "..") for part in pure.parts):
        raise EvidenceError("manifest output path is invalid")
    resolved_root = root.resolve(strict=True)
    parent = root.joinpath(*pure.parts[:-1])
    parent.mkdir(parents=True, exist_ok=True)
    current = resolved_root
    for part in pure.parts[:-1]:
        current = current / part
        mode = current.lstat().st_mode
        if stat.S_ISLNK(mode) or not stat.S_ISDIR(mode):
            raise EvidenceError("manifest output directory is unsafe")
    output = parent / pure.parts[-1]
    try:
        output_mode = output.lstat().st_mode
    except FileNotFoundError:
        pass
    else:
        if stat.S_ISLNK(output_mode) or not stat.S_ISREG(output_mode):
            raise EvidenceError("manifest output is not a regular file")
    return output


def _validate_artifact(
    repository_root: Path,
    evidence_directory: Path,
    artifact_path: str,
    expected_digest: str,
) -> None:
    if SHA256.fullmatch(expected_digest) is None:
        raise EvidenceError("artifact digest must use sha256:<64 lowercase hex>")
    relative = PurePosixPath(artifact_path)
    if (
        not artifact_path
        or "\\" in artifact_path
        or len(artifact_path) > 4096
        or any(ord(character) < 32 or ord(character) == 127 for character in artifact_path)
        or relative.is_absolute()
        or any(part in ("", ".", "..") for part in relative.parts)
    ):
        raise EvidenceError("artifact path must be a canonical repository-relative path")
    if relative.parts[0] != "sdk" or relative.suffix not in {".artifact", ".wasm"}:
        raise EvidenceError("promotion artifact must be a tracked SDK .artifact or .wasm file")
    repository_root = repository_root.resolve(strict=True)
    artifact = _regular_file(repository_root, artifact_path, MAX_PROMOTION_ARTIFACT_BYTES)
    hasher = hashlib.sha256()
    with artifact.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    actual = hasher.hexdigest()
    committed_artifact = _tracked_blob(repository_root, artifact_path)
    if hashlib.sha256(committed_artifact).hexdigest() != actual:
        raise EvidenceError("artifact working-tree bytes differ from the checked-out commit")
    if expected_digest != f"sha256:{actual}":
        raise EvidenceError("artifact digest does not match the checked-out file")

    package_manifest_path = (relative.parent / "pandora.package.json").as_posix()
    package_manifest_file = _regular_file(repository_root, package_manifest_path)
    committed_manifest = _tracked_blob(repository_root, package_manifest_path)
    manifest_bytes = package_manifest_file.read_bytes()
    if manifest_bytes != committed_manifest:
        raise EvidenceError("artifact manifest bytes differ from the checked-out commit")
    try:
        package_manifest = json.loads(manifest_bytes)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise EvidenceError("artifact package manifest is invalid JSON") from error
    if (
        not isinstance(package_manifest, dict)
        or package_manifest.get("content_hash") != expected_digest
        or not isinstance(package_manifest.get("id"), str)
        or not isinstance(package_manifest.get("version"), str)
    ):
        raise EvidenceError("artifact digest or identity does not match its tracked package manifest")
    package_lines = _regular_file(evidence_directory, "package-validation.jsonl").read_text(encoding="utf-8").splitlines()
    matching = []
    for line in package_lines:
        record = json.loads(line)
        package = record.get("package") if isinstance(record, dict) else None
        if (
            isinstance(package, dict)
            and package.get("id") == package_manifest["id"]
            and package.get("version") == package_manifest["version"]
            and package.get("content_hash") == expected_digest
        ):
            matching.append(record)
    if len(matching) != 1:
        raise EvidenceError("artifact is not validated exactly once by the pipeline evidence")


def verify_manifest(
    repository_root: Path,
    directory: Path,
    manifest_path: Path,
    commit: str,
    channel: str,
    expected_evidence_digest: str,
    artifact_path: str,
    expected_artifact_digest: str,
) -> None:
    _require_identity(commit, channel)
    _require_checked_out_commit(repository_root, commit)
    if SHA256.fullmatch(expected_evidence_digest) is None:
        raise EvidenceError("evidence digest must use sha256:<64 lowercase hex>")
    manifest_file = manifest_path.resolve(strict=True)
    if manifest_path.is_symlink() or not manifest_file.is_file():
        raise EvidenceError("evidence manifest must be a regular file")
    try:
        manifest_file.relative_to(directory.resolve(strict=True))
    except ValueError as error:
        raise EvidenceError("evidence manifest is outside the evidence directory") from error
    if manifest_file.stat().st_size > MAX_EVIDENCE_FILE_BYTES:
        raise EvidenceError("evidence manifest exceeds the size limit")
    try:
        manifest = json.loads(manifest_file.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise EvidenceError("evidence manifest is invalid JSON") from error
    if not isinstance(manifest, dict) or set(manifest) != {"schema_version", "commit", "channel", "files", "digest"}:
        raise EvidenceError("evidence manifest has an unsupported shape")
    if manifest["schema_version"] != 1 or manifest["commit"] != commit or manifest["channel"] != channel:
        raise EvidenceError("evidence manifest identity does not match the approved run")
    payload = evidence_payload(directory, commit, channel)
    if manifest["files"] != payload["files"]:
        raise EvidenceError("generated evidence files changed after manifest creation")
    actual_evidence_digest = digest_bytes(canonical_json(payload))
    if manifest["digest"] != actual_evidence_digest or expected_evidence_digest != actual_evidence_digest:
        raise EvidenceError("submitted evidence digest does not match the generated evidence")
    _validate_artifact(repository_root, directory, artifact_path, expected_artifact_digest)


def main() -> int:
    parser = argparse.ArgumentParser(description="Create and verify exact agent-pipeline evidence digests")
    subparsers = parser.add_subparsers(dest="command", required=True)
    create = subparsers.add_parser("create")
    create.add_argument("--directory", type=Path, required=True)
    create.add_argument("--repository-root", type=Path, required=True)
    create.add_argument("--commit", required=True)
    create.add_argument("--channel", required=True)
    verify = subparsers.add_parser("verify")
    verify.add_argument("--repository-root", type=Path, required=True)
    verify.add_argument("--directory", type=Path, required=True)
    verify.add_argument("--manifest", type=Path, required=True)
    verify.add_argument("--commit", required=True)
    verify.add_argument("--channel", required=True)
    verify.add_argument("--evidence-digest", required=True)
    verify.add_argument("--artifact-path", required=True)
    verify.add_argument("--artifact-digest", required=True)
    args = parser.parse_args()

    try:
        if args.command == "create":
            manifest = create_manifest(args.directory, args.commit, args.channel, args.repository_root)
            print(manifest["digest"])
        else:
            verify_manifest(
                args.repository_root,
                args.directory,
                args.manifest,
                args.commit,
                args.channel,
                args.evidence_digest,
                args.artifact_path,
                args.artifact_digest,
            )
            print("agent pipeline artifact and evidence digests verified")
        return 0
    except (EvidenceError, OSError, subprocess.SubprocessError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
