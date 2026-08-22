from __future__ import annotations

import hashlib
import re
from pathlib import Path
from urllib.parse import urlparse


_TARGETS = {
    ("linux", "x86_64"): "pandora-x86_64-unknown-linux-gnu",
    ("linux", "amd64"): "pandora-x86_64-unknown-linux-gnu",
    ("darwin", "x86_64"): "pandora-x86_64-apple-darwin",
    ("darwin", "amd64"): "pandora-x86_64-apple-darwin",
    ("darwin", "arm64"): "pandora-aarch64-apple-darwin",
    ("windows", "x86_64"): "pandora-x86_64-pc-windows-msvc.exe",
    ("windows", "amd64"): "pandora-x86_64-pc-windows-msvc.exe",
}
_DIGEST = re.compile(r"^[0-9a-fA-F]{64}$")
_VERSION = re.compile(r"^v[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$")


def artifact_name(platform: str, architecture: str) -> str:
    try:
        return _TARGETS[(platform.lower(), architecture.lower())]
    except KeyError as error:
        raise ValueError("unsupported Pandora platform or architecture") from error


def parse_checksums(text: str) -> dict[str, str]:
    manifest: dict[str, str] = {}
    for raw_line in text.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        parts = line.split()
        if len(parts) != 2 or not _DIGEST.fullmatch(parts[0]):
            raise ValueError("malformed checksum manifest")
        filename = parts[1].lstrip("*")
        if not filename or filename in manifest:
            raise ValueError("duplicate checksum entry")
        manifest[filename] = parts[0].lower()
    if not manifest:
        raise ValueError("empty checksum manifest")
    return manifest


def expected_checksum(manifest: dict[str, str], filename: str) -> str:
    for candidate in (filename, f"dist/{filename}"):
        if candidate in manifest:
            return manifest[candidate]
    raise ValueError(f"missing checksum for {filename}")


def verify_checksum(payload: bytes, expected: str) -> bool:
    return bool(_DIGEST.fullmatch(expected)) and hashlib.sha256(payload).hexdigest() == expected.lower()


def release_url(base: str, version: str, filename: str) -> str:
    if not _VERSION.fullmatch(version):
        raise ValueError("invalid Pandora release version")
    parsed = urlparse(base.rstrip("/"))
    if parsed.scheme != "https" or not parsed.netloc or parsed.username or parsed.password:
        raise ValueError("Pandora release URL must use HTTPS without credentials")
    if parsed.query or parsed.fragment:
        raise ValueError("Pandora release URL must not contain a query or fragment")
    if not filename or "/" in filename or "\\" in filename:
        raise ValueError("invalid Pandora release artifact name")
    return f"{base.rstrip('/')}/{version}/{filename}"


def cached_artifact(cache_dir: Path, filename: str, expected: str) -> Path | None:
    artifact = cache_dir / filename
    if not artifact.is_file() or not verify_checksum(artifact.read_bytes(), expected):
        return None
    return artifact
