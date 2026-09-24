from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from scripts.platform_signing_receipt import build_platform_signing_receipt
from scripts.release_evidence import ReleaseEvidenceError, build_release_evidence


COMMIT = "c" * 40
TAG = "v2.0.0-rc.1"
VENDOR_ARTIFACTS = {
    "pandora-x86_64-pc-windows-msvc.exe": "windows",
    "pandora-x86_64-apple-darwin": "macos",
    "desktop-windows-x64-Pandora.msi": "windows",
    "desktop-macos-x64-Pandora.dmg": "macos",
}


class ReleaseSigningEvidenceTests(unittest.TestCase):
    def make_dist(self, root: Path) -> Path:
        dist = root / "dist"
        dist.mkdir()
        artifacts = {
            "pandora-x86_64-unknown-linux-gnu": b"native linux\n",
            **{
                name: f"{name}\n".encode()
                for name in VENDOR_ARTIFACTS
            },
            "desktop-linux-x64-Pandora.AppImage": b"desktop linux\n",
            "install.sh": b"#!/bin/sh\n",
            "install.ps1": b"Write-Output pandora\n",
            "pandora-cli-2.0.0-rc.1.tgz": b"npm package\n",
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
        return dist

    def write_receipts(self, root: Path, dist: Path) -> list[Path]:
        receipt_directory = root / "receipts"
        receipt_directory.mkdir()
        receipts: list[Path] = []
        for artifact_name, platform in VENDOR_ARTIFACTS.items():
            receipt = build_platform_signing_receipt(
                release_tag=TAG,
                commit=COMMIT,
                platform=platform,
                artifact_path=dist / artifact_name,
                verifications=(
                    {"authenticode": True}
                    if platform == "windows"
                    else {"codesign": True, "notarization": True}
                ),
            )
            path = receipt_directory / f"{artifact_name}.receipt.json"
            path.write_text(json.dumps(receipt), encoding="utf-8")
            receipts.append(path)
        return receipts

    def test_rc_requires_a_commit_and_complete_vendor_receipts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = self.make_dist(root)

            with self.assertRaisesRegex(ReleaseEvidenceError, "commit"):
                build_release_evidence(TAG, dist, scope="full")

            with self.assertRaisesRegex(ReleaseEvidenceError, "receipt"):
                build_release_evidence(TAG, dist, scope="full", commit=COMMIT)

    def test_rc_evidence_uses_validated_receipt_digests_not_self_assertion(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = self.make_dist(root)
            receipts = self.write_receipts(root, dist)

            evidence = build_release_evidence(
                TAG,
                dist,
                scope="full",
                commit=COMMIT,
                signing_receipts=receipts,
            )

            self.assertEqual(evidence["platform_signing"]["windows_authenticode"], "verified_by_receipt")
            self.assertEqual(evidence["platform_signing"]["apple_codesign"], "verified_by_receipt")
            self.assertEqual(evidence["platform_signing"]["apple_notarization"], "verified_by_receipt")
            receipts = evidence["platform_signing"]["receipts"]
            self.assertEqual(len(receipts), 4)
            for receipt in receipts:
                self.assertRegex(receipt["file_sha256"], r"^[0-9a-f]{64}$")
                self.assertRegex(receipt["receipt_sha256"], r"^[0-9a-f]{64}$")
            self.assertNotIn("verified_in_build", json.dumps(evidence))

    def test_receipt_digest_and_commit_are_rechecked_against_the_artifact(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = self.make_dist(root)
            receipts = self.write_receipts(root, dist)
            (dist / "pandora-x86_64-pc-windows-msvc.exe").write_bytes(b"changed\n")

            with self.assertRaisesRegex(ReleaseEvidenceError, "digest mismatch"):
                build_release_evidence(
                    TAG,
                    dist,
                    scope="full",
                    commit=COMMIT,
                    signing_receipts=receipts,
                )

    def test_receipt_commit_mismatch_is_rejected_before_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = self.make_dist(root)
            receipts = self.write_receipts(root, dist)
            receipt = json.loads(receipts[0].read_text(encoding="utf-8"))
            receipt["commit"] = "d" * 40
            receipts[0].write_text(json.dumps(receipt), encoding="utf-8")

            with self.assertRaisesRegex(ReleaseEvidenceError, "commit"):
                build_release_evidence(
                    TAG,
                    dist,
                    scope="full",
                    commit=COMMIT,
                    signing_receipts=receipts,
                )

    def test_rejects_duplicate_and_non_vendor_receipt_references(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = self.make_dist(root)
            receipts = self.write_receipts(root, dist)

            with self.assertRaisesRegex(ReleaseEvidenceError, "duplicate"):
                build_release_evidence(
                    TAG,
                    dist,
                    scope="full",
                    commit=COMMIT,
                    signing_receipts=[receipts[0], receipts[0]],
                )

            non_vendor = json.loads(receipts[0].read_text(encoding="utf-8"))
            non_vendor["artifact"]["path"] = "install.sh"
            non_vendor_path = root / "non-vendor.json"
            non_vendor_path.write_text(json.dumps(non_vendor), encoding="utf-8")
            with self.assertRaisesRegex(ReleaseEvidenceError, "non-vendor"):
                build_release_evidence(
                    TAG,
                    dist,
                    scope="full",
                    commit=COMMIT,
                    signing_receipts=[non_vendor_path, *receipts[1:]],
                )

    def test_beta_does_not_require_vendor_receipts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = self.make_dist(root)

            evidence = build_release_evidence(
                "v2.0.0-beta.8", dist, scope="full"
            )

            self.assertFalse(evidence["platform_signing"]["required"])
            self.assertEqual(
                evidence["platform_signing"]["windows_authenticode"],
                "not_required",
            )
            self.assertEqual(evidence["platform_signing"]["receipts"], [])


if __name__ == "__main__":
    unittest.main()
