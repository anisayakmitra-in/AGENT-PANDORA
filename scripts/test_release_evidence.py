from __future__ import annotations

import hashlib
import tempfile
import unittest
from pathlib import Path

from scripts.release_evidence import ReleaseEvidenceError, build_release_evidence


class ReleaseEvidenceTests(unittest.TestCase):
    def _write_dist(self, root: Path) -> tuple[Path, dict[str, bytes]]:
        dist = root / "dist"
        dist.mkdir()
        artifacts = {
            "pandora-x86_64-unknown-linux-gnu": b"native cli\n",
            "desktop-linux-x64-pandora.AppImage": b"desktop bundle\n",
            "install.sh": b"#!/bin/sh\n",
            "install.ps1": b"Write-Output pandora\n",
            "pandora-cli-2.0.0-beta.7.tgz": b"npm package\n",
            "pandora-cargo-metadata.json": b"{}\n",
            "pandora.spdx.json": b"{}\n",
        }
        for name, payload in artifacts.items():
            (dist / name).write_bytes(payload)
        checksums = "\n".join(
            f"{hashlib.sha256(payload).hexdigest()}  {name}"
            for name, payload in sorted(artifacts.items())
        )
        (dist / "checksums.txt").write_text(f"{checksums}\n", encoding="utf-8")
        (dist / "checksums.txt.sig").write_bytes(b"cosign signature\n")
        (dist / "checksums.txt.pem").write_bytes(b"cosign certificate\n")
        (dist / "pandora-cargo-metadata.json").write_text("{}\n", encoding="utf-8")
        (dist / "pandora.spdx.json").write_text("{}\n", encoding="utf-8")
        return dist, artifacts

    def test_binds_checksums_signatures_sbom_provenance_and_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            dist, artifacts = self._write_dist(Path(temporary))

            evidence = build_release_evidence("v2.0.0-beta.7", dist)

            self.assertEqual(evidence["schema_version"], 1)
            self.assertEqual(evidence["release_tag"], "v2.0.0-beta.7")
            self.assertEqual(evidence["checksum_manifest"]["entries"], len(artifacts))
            self.assertTrue(evidence["signature"]["verified_in_workflow"])
            self.assertTrue(evidence["provenance"]["verified_in_workflow"])
            self.assertEqual(
                {item["path"] for item in evidence["artifacts"]},
                set(artifacts),
            )

    def test_rejects_an_artifact_changed_after_checksum_generation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            dist, _ = self._write_dist(Path(temporary))
            (dist / "desktop-linux-x64-pandora.AppImage").write_bytes(b"tampered\n")

            with self.assertRaisesRegex(ReleaseEvidenceError, "checksum mismatch"):
                build_release_evidence("v2.0.0-beta.7", dist)

    def test_requires_complete_signature_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            dist, _ = self._write_dist(Path(temporary))
            (dist / "checksums.txt.pem").unlink()

            with self.assertRaisesRegex(ReleaseEvidenceError, "missing or empty"):
                build_release_evidence("v2.0.0-beta.7", dist)

    def test_rejects_invalid_release_tag(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            dist, _ = self._write_dist(Path(temporary))

            with self.assertRaisesRegex(ReleaseEvidenceError, "invalid release tag"):
                build_release_evidence("latest", dist)


if __name__ == "__main__":
    unittest.main()
