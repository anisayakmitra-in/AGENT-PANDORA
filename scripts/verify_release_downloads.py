from __future__ import annotations

import argparse
import hashlib
from pathlib import Path

try:
    from .installer_contract import expected_checksum, parse_checksums
except ImportError:
    from installer_contract import expected_checksum, parse_checksums


class ReleaseDownloadError(ValueError):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def verify_release_downloads(manifest_path: Path, artifacts: list[Path]) -> None:
    if manifest_path.is_symlink() or not manifest_path.is_file():
        raise ReleaseDownloadError("checksum manifest must be a regular file")
    if not artifacts:
        raise ReleaseDownloadError("at least one release artifact is required")

    try:
        manifest = parse_checksums(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, ValueError) as error:
        raise ReleaseDownloadError(f"invalid checksum manifest: {error}") from error

    seen: set[str] = set()
    for artifact in artifacts:
        if artifact.is_symlink() or not artifact.is_file():
            raise ReleaseDownloadError(
                f"release artifact must be a regular file: {artifact}"
            )
        name = artifact.name
        if name in seen:
            raise ReleaseDownloadError(f"duplicate release artifact: {name}")
        seen.add(name)
        try:
            expected = expected_checksum(manifest, name)
        except ValueError as error:
            raise ReleaseDownloadError(str(error)) from error
        actual = sha256_file(artifact)
        if actual != expected:
            raise ReleaseDownloadError(
                f"checksum mismatch for {name}: expected {expected}, got {actual}"
            )


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Verify downloaded Pandora release artifacts against checksums.txt"
    )
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("artifacts", nargs="+", type=Path)
    arguments = parser.parse_args()

    try:
        verify_release_downloads(arguments.manifest, arguments.artifacts)
    except (OSError, ReleaseDownloadError) as error:
        print(f"error: {error}")
        return 1

    print(f"verified {len(arguments.artifacts)} downloaded release artifacts")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
