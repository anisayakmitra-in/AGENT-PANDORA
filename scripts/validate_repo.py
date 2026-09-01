from __future__ import annotations

import argparse
import hashlib
import json
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
GLIB_PATCH_ROOT = Path("third_party/glib-0.18.5-patched")
GLIB_PATCH_SOURCE = GLIB_PATCH_ROOT / "src" / "variant_iter.rs"
GLIB_PATCH_SOURCE_SHA256 = (
    "a0f5ee8acb8faa089bcdfbc9a57372609fce7654026ccef7d9a224d05a654ccc"
)
GLIB_PATCH_VCS_SHA = "42b9caf98e03ded086362d9653ca58fe94dc8658"
GLIB_PATCH_OVERRIDE = (
    'glib = { path = "../../../third_party/glib-0.18.5-patched" }'
)
GLIB_LOCK_PACKAGE = re.compile(
    r'\[\[package\]\]\r?\nname = "glib"\r?\nversion = "0\.18\.5"\r?\n'
    r"(?P<body>.*?)(?=\r?\n\[\[package\]\])",
    re.DOTALL,
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


def validate_patched_glib(root: Path) -> list[str]:
    findings: list[str] = []
    manifest = root / "apps" / "pandora-desktop" / "src-tauri" / "Cargo.toml"
    lockfile = root / "apps" / "pandora-desktop" / "src-tauri" / "Cargo.lock"
    source = root / GLIB_PATCH_SOURCE
    provenance = root / GLIB_PATCH_ROOT / ".cargo_vcs_info.json"
    patch_record = root / GLIB_PATCH_ROOT / "PANDORA-PATCH.md"

    if not manifest.is_file() or GLIB_PATCH_OVERRIDE not in manifest.read_text(
        encoding="utf-8"
    ):
        findings.append(
            f"{manifest.relative_to(root)}: patched glib source override is missing"
        )

    if not lockfile.is_file():
        findings.append(f"{lockfile.relative_to(root)}: desktop lockfile is missing")
    else:
        match = GLIB_LOCK_PACKAGE.search(lockfile.read_text(encoding="utf-8"))
        if match is None:
            findings.append(
                f"{lockfile.relative_to(root)}: patched glib lock entry is missing"
            )
        elif "source =" in match.group("body") or "checksum =" in match.group("body"):
            findings.append(
                f"{lockfile.relative_to(root)}: glib must resolve through the reviewed path override"
            )

    if not source.is_file():
        findings.append(f"{GLIB_PATCH_SOURCE}: patched source is missing")
    else:
        source_bytes = source.read_bytes()
        digest = hashlib.sha256(source_bytes).hexdigest()
        if digest != GLIB_PATCH_SOURCE_SHA256:
            findings.append(
                f"{GLIB_PATCH_SOURCE}: patched source digest does not match the reviewed fix"
            )
        source_text = source_bytes.decode("utf-8")
        if "let mut p: *mut libc::c_char" not in source_text or "&mut p," not in source_text:
            findings.append(
                f"{GLIB_PATCH_SOURCE}: mutable out-argument fix is missing"
            )
        if "let p: *mut libc::c_char" in source_text:
            findings.append(
                f"{GLIB_PATCH_SOURCE}: vulnerable immutable out-argument remains"
            )

    if not provenance.is_file():
        findings.append(
            f"{provenance.relative_to(root)}: crates.io source provenance is missing"
        )
    else:
        try:
            vcs = json.loads(provenance.read_text(encoding="utf-8"))
        except (json.JSONDecodeError, UnicodeDecodeError):
            findings.append(
                f"{provenance.relative_to(root)}: source provenance is invalid"
            )
        else:
            if vcs.get("git", {}).get("sha1") != GLIB_PATCH_VCS_SHA:
                findings.append(
                    f"{provenance.relative_to(root)}: source revision is not the reviewed crates.io revision"
                )

    if not patch_record.is_file():
        findings.append(
            f"{patch_record.relative_to(root)}: security patch record is missing"
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
    findings.extend(validate_patched_glib(root))
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
