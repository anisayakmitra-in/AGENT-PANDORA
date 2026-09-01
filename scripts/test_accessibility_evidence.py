from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from scripts.accessibility_evidence import (
    AccessibilityEvidenceError,
    CHECKS,
    PLATFORMS,
    validate_evidence_set,
)


COMMIT = "83a15e102ea4d7be42f33d3285ba5a88a37e8256"


class AccessibilityEvidenceTests(unittest.TestCase):
    def _write_evidence(self, root: Path) -> Path:
        evidence_root = root / "evidence"
        evidence_root.mkdir(parents=True)
        for platform, requirements in PLATFORMS.items():
            platform_root = evidence_root / platform
            platform_root.mkdir()
            assets = {
                "screen.png": b"portable screenshot bytes\n",
                "screen-reader.txt": f"{requirements['assistive_technology']} traversal passed\n".encode(),
                "lifecycle.log": b"install start update rollback uninstall passed\n",
            }
            kinds = {
                "screen.png": "screenshot",
                "screen-reader.txt": "screen-reader-notes",
                "lifecycle.log": "lifecycle-log",
            }
            for name, payload in assets.items():
                (platform_root / name).write_bytes(payload)
            record = {
                "schema_version": 1,
                "commit_sha": COMMIT,
                "artifact": {
                    "name": f"Pandora-{platform}.installer",
                    "sha256": hashlib.sha256(platform.encode()).hexdigest(),
                    "signed": False,
                    "notarized": False,
                },
                "platform": {
                    "id": platform,
                    "os_name": "Test OS",
                    "os_version": "1.0",
                    "architecture": requirements["architecture"],
                    "scales_tested": [100, 150, 200],
                    "minimum_window": {"width": 1080, "height": 720},
                },
                "assistive_technology": {
                    "name": requirements["assistive_technology"],
                    "version": "1.0",
                },
                "session": {
                    "tested_at": "2026-09-01T09:00:00Z",
                    "tester": "release-tester",
                    "packaged_app": True,
                    "clean_machine": True,
                },
                "release_identity": {
                    "desktop_version": "2.0.0-beta.7",
                    "cli_version": "2.0.0-beta.7",
                },
                "checks": {name: "pass" for name in CHECKS},
                "evidence": [
                    {
                        "path": f"{platform}/{name}",
                        "sha256": hashlib.sha256(payload).hexdigest(),
                        "kind": kinds[name],
                    }
                    for name, payload in assets.items()
                ],
                "findings": [],
            }
            (evidence_root / f"{platform}.json").write_text(
                json.dumps(record, indent=2) + "\n", encoding="utf-8"
            )
        return evidence_root

    def test_accepts_one_exact_complete_record_per_advertised_platform(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence_root = self._write_evidence(root)

            index = validate_evidence_set(evidence_root, COMMIT, root)

            self.assertTrue(index["complete"])
            self.assertEqual(index["commit_sha"], COMMIT)
            self.assertEqual(index["release_identity"], "2.0.0-beta.7")
            self.assertEqual(
                {record["platform"] for record in index["platforms"]}, set(PLATFORMS)
            )

    def test_rejects_a_missing_platform_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence_root = self._write_evidence(root)
            (evidence_root / "macos-arm64.json").unlink()

            with self.assertRaisesRegex(
                AccessibilityEvidenceError, "exactly one root manifest"
            ):
                validate_evidence_set(evidence_root, COMMIT, root)

    def test_rejects_commit_or_release_identity_substitution(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence_root = self._write_evidence(root)

            with self.assertRaisesRegex(AccessibilityEvidenceError, "exact commit"):
                validate_evidence_set(evidence_root, "f" * 40, root)

            manifest = evidence_root / "linux-x64.json"
            record = json.loads(manifest.read_text(encoding="utf-8"))
            record["release_identity"]["cli_version"] = "2.0.0-beta.6"
            manifest.write_text(json.dumps(record), encoding="utf-8")
            with self.assertRaisesRegex(AccessibilityEvidenceError, "release identities"):
                validate_evidence_set(evidence_root, COMMIT, root)

    def test_rejects_tampered_or_traversing_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence_root = self._write_evidence(root)
            (evidence_root / "windows-x64" / "screen-reader.txt").write_text(
                "changed after review\n", encoding="utf-8"
            )
            with self.assertRaisesRegex(AccessibilityEvidenceError, "checksum mismatch"):
                validate_evidence_set(evidence_root, COMMIT, root)

            evidence_root = self._write_evidence(root / "second")
            manifest = evidence_root / "windows-x64.json"
            record = json.loads(manifest.read_text(encoding="utf-8"))
            record["evidence"][0]["path"] = "../outside.png"
            manifest.write_text(json.dumps(record), encoding="utf-8")
            with self.assertRaisesRegex(AccessibilityEvidenceError, "escapes"):
                validate_evidence_set(evidence_root, COMMIT, root / "second")

    def test_rejects_wrong_assistive_technology_and_failed_checks(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            evidence_root = self._write_evidence(root)
            manifest = evidence_root / "windows-x64.json"
            record = json.loads(manifest.read_text(encoding="utf-8"))
            record["assistive_technology"]["name"] = "Narrator"
            manifest.write_text(json.dumps(record), encoding="utf-8")
            with self.assertRaisesRegex(AccessibilityEvidenceError, "must use NVDA"):
                validate_evidence_set(evidence_root, COMMIT, root)

            evidence_root = self._write_evidence(root / "second")
            manifest = evidence_root / "linux-x64.json"
            record = json.loads(manifest.read_text(encoding="utf-8"))
            record["checks"]["visible_focus"] = "fail"
            manifest.write_text(json.dumps(record), encoding="utf-8")
            with self.assertRaisesRegex(AccessibilityEvidenceError, "failed checks"):
                validate_evidence_set(evidence_root, COMMIT, root / "second")


if __name__ == "__main__":
    unittest.main()
