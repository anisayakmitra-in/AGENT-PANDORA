from __future__ import annotations

import copy
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from scripts.external_signing_request import (
    ExternalSigningRequestError,
    build_external_signing_request,
    signing_request_digest,
    validate_external_signing_request,
)


ROOT = Path(__file__).resolve().parents[1]
VALIDATOR = ROOT / "scripts" / "external_signing_request.py"
COMMIT = "a" * 40
TAG = "v2.0.0-rc.1"


class ExternalSigningRequestTests(unittest.TestCase):
    def make_artifact(self, root: Path, name: str) -> Path:
        path = root / name
        path.write_bytes((name + "\n").encode("utf-8"))
        return path

    def test_request_is_deterministic_and_binds_exact_artifact_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dmg = self.make_artifact(root, "Pandora.dmg")
            first = build_external_signing_request(
                release_tag=TAG,
                source_commit=COMMIT,
                target="aarch64-apple-darwin",
                platform="macos",
                artifacts=[(dmg, "dmg")],
            )
            second = build_external_signing_request(
                release_tag=TAG,
                source_commit=COMMIT,
                target="aarch64-apple-darwin",
                platform="macos",
                artifacts=[(dmg, "dmg")],
            )

            self.assertEqual(first, second)
            self.assertEqual(first["request_id"], signing_request_digest(first))
            self.assertEqual(first["policy"]["source_execution"], "forbidden")
            self.assertEqual(first["policy"]["rebuild"], "forbidden")
            self.assertEqual(
                first["required_checks"],
                ["codesign", "notarization"],
            )
            self.assertEqual(
                validate_external_signing_request(first, artifact_root=root),
                first,
            )

    def test_windows_request_requires_authenticode_and_rejects_wrong_kind(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            msi = self.make_artifact(root, "Pandora.msi")
            request = build_external_signing_request(
                release_tag=TAG,
                source_commit=COMMIT,
                target="x86_64-pc-windows-msvc",
                platform="windows",
                artifacts=[(msi, "msi")],
            )
            self.assertEqual(request["required_checks"], ["authenticode"])

            with self.assertRaisesRegex(ExternalSigningRequestError, "platform"):
                build_external_signing_request(
                    release_tag=TAG,
                    source_commit=COMMIT,
                    target="x86_64-pc-windows-msvc",
                    platform="windows",
                    artifacts=[(msi, "dmg")],
                )

            with self.assertRaisesRegex(ExternalSigningRequestError, "target"):
                build_external_signing_request(
                    release_tag=TAG,
                    source_commit=COMMIT,
                    target="x86_64-apple-darwin",
                    platform="windows",
                    artifacts=[(msi, "msi")],
                )

    def test_validation_rejects_unknown_fields_and_source_execution_policy(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            artifact = self.make_artifact(root, "Pandora.dmg")
            request = build_external_signing_request(
                release_tag=TAG,
                source_commit=COMMIT,
                target="x86_64-apple-darwin",
                platform="macos",
                artifacts=[(artifact, "dmg")],
            )

            unknown = copy.deepcopy(request)
            unknown["secret"] = "must-not-be-accepted"
            with self.assertRaisesRegex(ExternalSigningRequestError, "shape"):
                validate_external_signing_request(unknown)

            executable = copy.deepcopy(request)
            executable["policy"]["source_execution"] = "allowed"
            with self.assertRaisesRegex(ExternalSigningRequestError, "source execution"):
                validate_external_signing_request(executable)

            boolean_schema = copy.deepcopy(request)
            boolean_schema["schema_version"] = True
            with self.assertRaisesRegex(ExternalSigningRequestError, "schema"):
                validate_external_signing_request(boolean_schema)

            boolean_count = copy.deepcopy(request)
            boolean_count["policy"]["artifact_count"] = True
            with self.assertRaisesRegex(ExternalSigningRequestError, "artifact_count"):
                validate_external_signing_request(boolean_count)

            unsafe_name = copy.deepcopy(request)
            unsafe_name["artifacts"][0]["name"] = "../Pandora.dmg"
            unsafe_name["request_id"] = signing_request_digest(unsafe_name)
            with self.assertRaisesRegex(ExternalSigningRequestError, "safe file name"):
                validate_external_signing_request(unsafe_name)

    def test_validation_rejects_artifact_substitution_and_duplicate_names(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            first = self.make_artifact(root, "Pandora.dmg")
            second = self.make_artifact(root, "Other.dmg")
            request = build_external_signing_request(
                release_tag=TAG,
                source_commit=COMMIT,
                target="x86_64-apple-darwin",
                platform="macos",
                artifacts=[(first, "dmg"), (second, "dmg")],
            )
            first.write_bytes(b"changed\n")
            with self.assertRaisesRegex(ExternalSigningRequestError, "binding mismatch"):
                validate_external_signing_request(request, artifact_root=root)

            duplicate = copy.deepcopy(request)
            duplicate["artifacts"][1]["name"] = duplicate["artifacts"][0]["name"]
            duplicate["request_id"] = signing_request_digest(duplicate)
            with self.assertRaisesRegex(ExternalSigningRequestError, "duplicate"):
                validate_external_signing_request(duplicate)

    def test_command_validates_a_serialized_request(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            artifact = self.make_artifact(root, "Pandora.dmg")
            request = build_external_signing_request(
                release_tag=TAG,
                source_commit=COMMIT,
                target="x86_64-apple-darwin",
                platform="macos",
                artifacts=[(artifact, "dmg")],
            )
            request_path = root / "request.json"
            request_path.write_text(json.dumps(request), encoding="utf-8")
            result = subprocess.run(
                [
                    sys.executable,
                    str(VALIDATOR),
                    str(request_path),
                    "--artifacts-root",
                    str(root),
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("external signing request validated", result.stdout)

            request_path.write_text(
                '{"schema_version": 1, "schema_version": 1}',
                encoding="utf-8",
            )
            result = subprocess.run(
                [
                    sys.executable,
                    str(VALIDATOR),
                    str(request_path),
                ],
                check=False,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("duplicate JSON key", result.stderr)


if __name__ == "__main__":
    unittest.main()
