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


def validate_content(relative: Path, content: bytes) -> list[str]:
    if PRIVATE_KEY_MARKER.search(content):
        return [f"{relative}: private key content must not be tracked"]
    return []


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


def tracked_paths(root: Path) -> list[Path]:
    result = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z"],
        check=True,
        capture_output=True,
    )
    return [Path(path) for path in result.stdout.decode("utf-8").split("\0") if path]


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate tracked Pandora repository files")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    arguments = parser.parse_args()
    root = arguments.root.resolve()
    findings = validate_paths(root, tracked_paths(root))

    if findings:
        for finding in findings:
            print(f"error: {finding}")
        return 1

    print("repository validation passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
