from __future__ import annotations

import copy
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from scripts.platform_signing_receipt import (
    SigningReceiptError,
    build_platform_signing_receipt,
    receipt_digest,
    validate_platform_signing_receipt,
)


ROOT = Path(__file__).resolve().parents[1]
VALIDATOR = ROOT / "scripts" / "platform_signing_receipt.py"
COMMIT = "a" * 40
TAG = "v2.0.0-rc.1"


class PlatformSigningReceiptTests(unittest.TestCase):
    def make_artifact(self, root: Path, name: str = "pandora.exe") -> Path:
        artifact = root / name
        artifact.write_bytes(b"signed artifact\n")
        return artifact

    def test_windows_receipt_binds_tag_commit_artifact_digest_and_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            artifact = self.make_artifact(Path(temporary))
            receipt = build_platform_signing_receipt(
                release_tag=TAG,
                commit=COMMIT,
                platform="windows",
                artifact_path=artifact,
                verifications={"authenticode": True},
            )

            validated = validate_platform_signing_receipt(
                receipt,
                artifact,
                expected_tag=TAG,
                expected_commit=COMMIT,
            )

            self.assertEqual(validated["artifact"]["path"], artifact.name)
            self.assertEqual(
                validated["artifact"]["sha256"],
                hashlib.sha256(artifact.read_bytes()).hexdigest(),
            )
            self.assertEqual(validated["artifact"]["bytes"], artifact.stat().st_size)
            self.assertEqual(validated["checks"][0]["verifier"], "signtool-verify-v1")
            validated["checks"][0]["verified"] = False
            self.assertTrue(receipt["checks"][0]["verified"])

    def test_macos_receipt_requires_codesign_and_notarization_results(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            artifact = self.make_artifact(Path(temporary), "Pandora.app")
            receipt = build_platform_signing_receipt(
                release_tag=TAG,
                commit=COMMIT,
                platform="macos",
                artifact_path=artifact,
                verifications={"codesign": True, "notarization": True},
            )

            validated = validate_platform_signing_receipt(
                receipt,
                artifact,
                expected_tag=TAG,
                expected_commit=COMMIT,
            )

            self.assertEqual(
                [check["name"] for check in validated["checks"]],
                ["codesign", "notarization"],
            )
            self.assertEqual(
                [check["verifier"] for check in validated["checks"]],
                ["codesign-verify-v1", "stapler-validate-v1"],
            )

    def test_failure_receipt_is_valid_evidence_but_not_successful(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            artifact = self.make_artifact(Path(temporary))
            receipt = build_platform_signing_receipt(
                release_tag=TAG,
                commit=COMMIT,
                platform="windows",
                artifact_path=artifact,
                verifications={"authenticode": False},
            )

            with self.assertRaisesRegex(SigningReceiptError, "not verified"):
                validate_platform_signing_receipt(
                    receipt,
                    artifact,
                    expected_tag=TAG,
                    expected_commit=COMMIT,
                )

            validated = validate_platform_signing_receipt(
                receipt,
                artifact,
                expected_tag=TAG,
                expected_commit=COMMIT,
                require_verified=False,
            )
            self.assertFalse(validated["checks"][0]["verified"])

    def test_rejects_digest_byte_tag_commit_and_path_substitution(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            artifact = self.make_artifact(root)
            receipt = build_platform_signing_receipt(
                release_tag=TAG,
                commit=COMMIT,
                platform="windows",
                artifact_path=artifact,
                verifications={"authenticode": True},
            )

            cases = (
                ("digest", {"artifact": {**receipt["artifact"], "sha256": "0" * 64}}, artifact, TAG, COMMIT),
                ("bytes", {"artifact": {**receipt["artifact"], "bytes": 1}}, artifact, TAG, COMMIT),
                ("tag", None, artifact, "v2.0.0", COMMIT),
                ("commit", None, artifact, TAG, "b" * 40),
                ("path", {"artifact": {**receipt["artifact"], "path": "../Pandora.exe"}}, artifact, TAG, COMMIT),
            )
            for label, replacement, checked_artifact, checked_tag, checked_commit in cases:
                with self.subTest(label=label):
                    candidate = copy.deepcopy(receipt)
                    if replacement is not None:
                        candidate.update(replacement)
                    with self.assertRaises(SigningReceiptError):
                        validate_platform_signing_receipt(
                            candidate,
                            checked_artifact,
                            expected_tag=checked_tag,
                            expected_commit=checked_commit,
                        )

    def test_rejects_unknown_shape_wrong_platform_and_missing_checks(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            artifact = self.make_artifact(Path(temporary))
            receipt = build_platform_signing_receipt(
                release_tag=TAG,
                commit=COMMIT,
                platform="windows",
                artifact_path=artifact,
                verifications={"authenticode": True},
            )

            invalid = []
            unknown = copy.deepcopy(receipt)
            unknown["unexpected"] = True
            invalid.append(unknown)
            wrong_platform = copy.deepcopy(receipt)
            wrong_platform["platform"] = "linux"
            invalid.append(wrong_platform)
            missing = copy.deepcopy(receipt)
            missing["checks"] = []
            invalid.append(missing)
            wrong_verifier = copy.deepcopy(receipt)
            wrong_verifier["checks"][0]["verifier"] = "certs-only"
            invalid.append(wrong_verifier)
            boolean_schema = copy.deepcopy(receipt)
            boolean_schema["schema_version"] = True
            invalid.append(boolean_schema)
            wrong_commit_length = copy.deepcopy(receipt)
            wrong_commit_length["commit"] = "a" * 41
            invalid.append(wrong_commit_length)

            for candidate in invalid:
                with self.subTest(candidate=candidate):
                    with self.assertRaises(SigningReceiptError):
                        validate_platform_signing_receipt(
                            candidate,
                            artifact,
                            expected_tag=TAG,
                            expected_commit=COMMIT,
                        )

    def test_receipt_digest_is_canonical_for_key_order(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            artifact = self.make_artifact(Path(temporary))
            receipt = build_platform_signing_receipt(
                release_tag=TAG,
                commit=COMMIT,
                platform="windows",
                artifact_path=artifact,
                verifications={"authenticode": True},
            )
            reordered = dict(reversed(list(receipt.items())))

            self.assertEqual(receipt_digest(receipt), receipt_digest(reordered))

    def test_command_validates_receipt_and_rejects_tampered_artifact(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            artifact = self.make_artifact(root)
            receipt = build_platform_signing_receipt(
                release_tag=TAG,
                commit=COMMIT,
                platform="windows",
                artifact_path=artifact,
                verifications={"authenticode": True},
            )
            receipt_path = root / "receipt.json"
            receipt_path.write_text(json.dumps(receipt), encoding="utf-8")
            environment = {
                **os.environ,
                "PATH": f"C:\\Program Files\\Git\\cmd;{os.environ.get('PATH', '')}",
            }
            command = [
                sys.executable,
                str(VALIDATOR),
                str(receipt_path),
                "--artifact",
                str(artifact),
                "--tag",
                TAG,
                "--commit",
                COMMIT,
            ]
            result = subprocess.run(
                command,
                check=False,
                capture_output=True,
                text=True,
                env=environment,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("platform signing receipt verified", result.stdout)

            receipt_path.write_text(
                '{"schema_version": 1, "schema_version": 1}',
                encoding="utf-8",
            )
            result = subprocess.run(
                command,
                check=False,
                capture_output=True,
                text=True,
                env=environment,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("duplicate JSON key", result.stderr)

            receipt_path.write_text(json.dumps(receipt), encoding="utf-8")
            artifact.write_bytes(b"changed\n")
            result = subprocess.run(
                command,
                check=False,
                capture_output=True,
                text=True,
                env=environment,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("digest mismatch", result.stderr)


if __name__ == "__main__":
    unittest.main()
