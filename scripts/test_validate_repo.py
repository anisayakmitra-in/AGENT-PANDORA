import unittest
from pathlib import Path

from scripts.validate_repo import validate_content, validate_paths


class ValidateRepositoryTests(unittest.TestCase):
    def test_clean_fixture_passes(self) -> None:
        self.assertEqual(validate_paths(Path("."), [Path("README.md")]), [])

    def test_rejects_tracked_build_output(self) -> None:
        findings = validate_paths(Path("."), [Path("target/debug/pandora.exe")])

        self.assertTrue(any("generated" in finding for finding in findings))

    def test_rejects_env_file_without_exposing_value(self) -> None:
        findings = validate_paths(Path("."), [Path(".env")])

        rendered = "\n".join(findings)
        self.assertTrue(any("credential" in finding for finding in findings))
        self.assertNotIn("sk-test-value-that-must-not-print", rendered)

    def test_rejects_private_key_content_without_exposing_key(self) -> None:
        private_key = "-----BEGIN PRIVATE KEY-----\nsecret\n-----END PRIVATE KEY-----\n"
        findings = validate_content(Path("signing.key"), private_key.encode("utf-8"))

        rendered = "\n".join(findings)
        self.assertTrue(any("private key" in finding for finding in findings))
        self.assertNotIn("secret", rendered)


if __name__ == "__main__":
    unittest.main()
