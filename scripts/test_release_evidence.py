from __future__ import annotations

import hashlib
import tempfile
import unittest
from pathlib import Path

from scripts.release_evidence import (
    ReleaseEvidenceError,
    build_release_evidence,
    platform_signing_required,
)


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
        # Keep fixture bytes identical on POSIX and Windows. Text-mode writes
        # normalize newlines on Windows after the checksum manifest is built.
        (dist / "pandora-cargo-metadata.json").write_bytes(b"{}\n")
        (dist / "pandora.spdx.json").write_bytes(b"{}\n")
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
            self.assertFalse(evidence["platform_signing"]["required"])
            self.assertEqual(
                evidence["platform_signing"]["windows_authenticode"], "not_required"
            )
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

    def test_release_candidate_and_stable_require_vendor_platform_signing(self) -> None:
        self.assertFalse(platform_signing_required("v2.0.0-beta.7"))
        self.assertTrue(platform_signing_required("v2.0.0-rc.1"))
        self.assertTrue(platform_signing_required("v2.0.0"))

        with tempfile.TemporaryDirectory() as temporary:
            dist, _ = self._write_dist(Path(temporary))
            evidence = build_release_evidence("v2.0.0-rc.1", dist)
            self.assertTrue(evidence["platform_signing"]["required"])
            self.assertEqual(
                evidence["platform_signing"]["apple_notarization"],
                "verified_in_build",
            )


if __name__ == "__main__":
    unittest.main()
