from __future__ import annotations

import argparse
import json
import re
from pathlib import Path


STABLE_TAG = re.compile(r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$")
RELEASE_HEADING = re.compile(r"^##\s+(v\S+)\s*$")
COMMIT_SHA = re.compile(r"^[0-9a-f]{40}$")


class StableRollbackEvidenceError(ValueError):
    pass


def parse_stable_tag(tag: str) -> tuple[int, int, int]:
    match = STABLE_TAG.fullmatch(tag)
    if match is None:
        raise StableRollbackEvidenceError(f"stable rollback requires a stable tag: {tag}")
    return tuple(int(match.group(index)) for index in range(1, 4))


def build_stable_rollback_evidence(
    changelog: str,
    current_tag: str,
    commit_sha: str,
    workflow_run_id: str,
) -> dict[str, object]:
    current = parse_stable_tag(current_tag)
    if COMMIT_SHA.fullmatch(commit_sha) is None:
        raise StableRollbackEvidenceError("stable rollback evidence requires an exact commit SHA")
    if not workflow_run_id.isdigit() or int(workflow_run_id) <= 0:
        raise StableRollbackEvidenceError("stable rollback evidence requires a workflow run ID")

    stable_releases: dict[str, tuple[int, int, int]] = {}
    for line in changelog.splitlines():
        heading = RELEASE_HEADING.fullmatch(line)
        if heading is None:
            continue
        tag = heading.group(1)
        match = STABLE_TAG.fullmatch(tag)
        if match is None:
            continue
        if tag in stable_releases:
            raise StableRollbackEvidenceError(f"duplicate stable release section: {tag}")
        stable_releases[tag] = parse_stable_tag(tag)
    if current_tag not in stable_releases:
        raise StableRollbackEvidenceError(
            f"current stable release section is missing: {current_tag}"
        )

    compatible = [
        (tag, version)
        for tag, version in stable_releases.items()
        if version[:2] == current[:2] and version < current
    ]
    if not compatible:
        if current[2] != 0:
            raise StableRollbackEvidenceError(
                f"stable patch {current_tag} has no compatible stable predecessor"
            )
        return {
            "schema_version": 1,
            "complete": False,
            "state": "pending_first_patch",
            "release_tag": current_tag,
            "commit_sha": commit_sha,
            "predecessor_tag": None,
            "workflow_run_id": int(workflow_run_id),
            "reason": "the first stable release in this line has no older compatible stable artifact",
            "closure_release": f"v{current[0]}.{current[1]}.1",
        }

    predecessor_tag, _ = max(compatible, key=lambda item: item[1])
    return {
        "schema_version": 1,
        "complete": True,
        "state": "verified",
        "release_tag": current_tag,
        "commit_sha": commit_sha,
        "predecessor_tag": predecessor_tag,
        "workflow_run_id": int(workflow_run_id),
        "published_lifecycle_jobs": [
            "smoke-install",
            "smoke-desktop",
            "stable-desktop-rollback",
        ],
        "install_update_backup_restore_rollback_uninstall": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Record stable-to-stable rollback closure after published lifecycle jobs"
    )
    parser.add_argument("tag")
    parser.add_argument("--commit", required=True)
    parser.add_argument("--workflow-run-id", required=True)
    parser.add_argument(
        "--changelog",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "CHANGELOG.md",
    )
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    try:
        evidence = build_stable_rollback_evidence(
            arguments.changelog.read_text(encoding="utf-8"),
            arguments.tag,
            arguments.commit,
            arguments.workflow_run_id,
        )
        arguments.output.parent.mkdir(parents=True, exist_ok=True)
        arguments.output.write_text(
            json.dumps(evidence, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
    except (OSError, StableRollbackEvidenceError) as error:
        print(f"stable rollback evidence: {error}")
        return 1
    print(f"stable rollback evidence written to {arguments.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
