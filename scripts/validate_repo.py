from __future__ import annotations

import argparse
import re
import subprocess
from pathlib import Path


GENERATED_PARTS = {
    ".scan-staging",
    ".venv",
    "__pycache__",
    "build",
    "dist",
    "node_modules",
    "target",
}
CREDENTIAL_FILENAMES = {".env", ".env.local", ".env.production", ".env.test"}
CREDENTIAL_SUFFIXES = {".key", ".pem", ".p12", ".pfx"}
PRIVATE_KEY_MARKER = re.compile(rb"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----")
ACTION_REFERENCE = re.compile(rb"(?m)^\s*-\s+uses:\s*([^\s#]+)")
COMMIT_SHA = re.compile(rb"[0-9a-fA-F]{40}")
RELEASE_TRAIN_TAG = re.compile(
    r"^v[0-9]+\.[0-9]+\.[0-9]+(?:-(?:alpha|beta|rc)\.[0-9]+)?$"
)
CHANGELOG_RELEASE_HEADING = re.compile(r"(?m)^##\s+(v\S+)\s*$")
WORKER_SOAK_PROFILES = (
    "ten-minute",
    "two-hour",
    "eight-hour",
    "twenty-four-hour",
)
WORKER_SOAK_PLATFORMS = (
    "linux-x64",
    "macos-x64",
    "macos-arm64",
    "windows-x64",
)


def validate_content(relative: Path, content: bytes) -> list[str]:
    findings: list[str] = []
    if PRIVATE_KEY_MARKER.search(content):
        findings.append(f"{relative}: private key content must not be tracked")

    if relative.parts[:2] == (".github", "workflows") and relative.suffix in {
        ".yaml",
        ".yml",
    }:
        for reference in ACTION_REFERENCE.findall(content):
            if reference.startswith((b"./", b"docker://")):
                continue
            _, separator, revision = reference.rpartition(b"@")
            if not separator or COMMIT_SHA.fullmatch(revision) is None:
                findings.append(
                    f"{relative}: GitHub Actions must use a full commit SHA"
                )

    return findings


def validate_paths(root: Path, paths: list[Path]) -> list[str]:
    findings: list[str] = []

    for relative in sorted(paths, key=lambda path: path.as_posix()):
        if relative.is_absolute():
            findings.append(f"{relative}: absolute repository path is not allowed")
            continue

        components = {part.lower() for part in relative.parts}
        name = relative.name.lower()
        if components & GENERATED_PARTS:
            findings.append(f"{relative}: generated artifact must not be tracked")

        if name in CREDENTIAL_FILENAMES or relative.suffix.lower() in CREDENTIAL_SUFFIXES:
            findings.append(f"{relative}: credential-like file must not be tracked")

        candidate = root.joinpath(*relative.parts)
        if candidate.is_file():
            findings.extend(validate_content(relative, candidate.read_bytes()[:65536]))

    return findings


def validate_release_changelog(changelog: str, tags: list[str]) -> list[str]:
    findings: list[str] = []
    headings = list(CHANGELOG_RELEASE_HEADING.finditer(changelog))
    for tag in sorted(set(tags)):
        if RELEASE_TRAIN_TAG.fullmatch(tag) is None:
            continue
        matching = [heading for heading in headings if heading.group(1) == tag]
        if not matching:
            findings.append(f"CHANGELOG.md: release section {tag} is missing")
            continue
        if len(matching) > 1:
            findings.append(f"CHANGELOG.md: release section {tag} is duplicated")
            continue
        heading = matching[0]
        following = next(
            (
                candidate
                for candidate in headings
                if candidate.start() > heading.start()
            ),
            None,
        )
        body_end = following.start() if following is not None else len(changelog)
        if not changelog[heading.end() : body_end].strip():
            findings.append(f"CHANGELOG.md: release section {tag} is empty")
    return findings


def validate_worker_soak_workflows(root: Path) -> list[str]:
    findings: list[str] = []
    campaign_path = root / ".github" / "workflows" / "worker-soak.yml"
    segment_path = root / ".github" / "workflows" / "worker-soak-segment.yml"
    runner_path = root / "scripts" / "worker_soak.py"
    for path in (campaign_path, segment_path, runner_path):
        if not path.is_file():
            findings.append(f"{path.relative_to(root)}: worker soak component is missing")
    if findings:
        return findings

    campaign = campaign_path.read_text(encoding="utf-8")
    segment = segment_path.read_text(encoding="utf-8")
    runner = runner_path.read_text(encoding="utf-8")
    for profile in WORKER_SOAK_PROFILES:
        if profile not in campaign or f'"{profile}"' not in runner:
            findings.append(
                f"{campaign_path.relative_to(root)}: worker soak profile {profile} is incomplete"
            )
    if campaign.count("uses: ./.github/workflows/worker-soak-segment.yml") != 6:
        findings.append(
            f"{campaign_path.relative_to(root)}: six checkpointed segment jobs are required"
        )
    for numeric_input in ("jobs", "producers", "rounds"):
        conversion = f"fromJSON(inputs.{numeric_input})"
        if campaign.count(conversion) != 6:
            findings.append(
                f"{campaign_path.relative_to(root)}: dispatch input {numeric_input} "
                "must be converted for all six numeric reusable-workflow inputs"
            )
    for dependency in ("needs: segment_1", "needs: segment_2", "needs: segment_3", "needs: segment_4", "needs: segment_5"):
        if dependency not in campaign:
            findings.append(
                f"{campaign_path.relative_to(root)}: sequential checkpoint {dependency} is missing"
            )
    for platform in WORKER_SOAK_PLATFORMS:
        if f"platform: {platform}" not in segment:
            findings.append(
                f"{segment_path.relative_to(root)}: platform {platform} is missing"
            )
    for required in (
        "timeout-minutes: 345",
        "scripts/worker_soak.py run-segment",
        "if: always()",
        "retention-days: 90",
    ):
        if required not in segment:
            findings.append(
                f"{segment_path.relative_to(root)}: required evidence control {required} is missing"
            )
    for required in (
        "scripts/worker_soak.py aggregate",
        "merge-multiple: true",
        "worker-soak-campaign",
    ):
        if required not in campaign:
            findings.append(
                f"{campaign_path.relative_to(root)}: required campaign control {required} is missing"
            )
    return findings


def tracked_paths(root: Path) -> list[Path]:
    result = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"],
        check=True,
        capture_output=True,
    )
    return [Path(path) for path in result.stdout.decode("utf-8").split("\0") if path]


def repository_tags(root: Path) -> list[str]:
    result = subprocess.run(
        ["git", "-C", str(root), "tag", "--list"],
        check=True,
        capture_output=True,
        text=True,
    )
    return [tag for tag in result.stdout.splitlines() if tag]


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate tracked Pandora repository files")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    arguments = parser.parse_args()
    root = arguments.root.resolve()
    findings = validate_paths(root, tracked_paths(root))
    findings.extend(validate_worker_soak_workflows(root))
    changelog_path = root / "CHANGELOG.md"
    if not changelog_path.is_file():
        findings.append("CHANGELOG.md: tracked changelog is missing")
    else:
        findings.extend(
            validate_release_changelog(
                changelog_path.read_text(encoding="utf-8"),
                repository_tags(root),
            )
        )

    if findings:
        for finding in findings:
            print(f"error: {finding}")
        return 1

    print("repository validation passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
