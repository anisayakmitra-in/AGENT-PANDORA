from __future__ import annotations

import hashlib
import tempfile
import unittest
from pathlib import Path

from scripts.verify_release_downloads import (
    ReleaseDownloadError,
    verify_release_downloads,
)


class VerifyReleaseDownloadsTests(unittest.TestCase):
    def _fixture(self, root: Path) -> tuple[Path, list[Path]]:
        artifacts = [
            root / "pandora-x86_64-unknown-linux-gnu",
            root / "desktop-linux-x64-Pandora_2.0.0_amd64.deb",
        ]
        payloads = [b"native sidecar\n", b"desktop package\n"]
        for path, payload in zip(artifacts, payloads, strict=True):
            path.write_bytes(payload)
        manifest = root / "checksums.txt"
        manifest.write_text(
            "".join(
                f"{hashlib.sha256(payload).hexdigest()}  {path.name}\n"
                for path, payload in zip(artifacts, payloads, strict=True)
            ),
            encoding="utf-8",
        )
        return manifest, artifacts

    def test_verifies_native_and_desktop_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._fixture(Path(temporary))

            verify_release_downloads(manifest, artifacts)

    def test_rejects_changed_artifact(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._fixture(Path(temporary))
            artifacts[1].write_bytes(b"changed\n")

            with self.assertRaisesRegex(ReleaseDownloadError, "checksum mismatch"):
                verify_release_downloads(manifest, artifacts)

    def test_rejects_missing_and_duplicate_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._fixture(Path(temporary))

            with self.assertRaisesRegex(ReleaseDownloadError, "regular file"):
                verify_release_downloads(manifest, [Path(temporary) / "missing"])
            with self.assertRaisesRegex(ReleaseDownloadError, "duplicate"):
                verify_release_downloads(manifest, [artifacts[0], artifacts[0]])


if __name__ == "__main__":
    unittest.main()
