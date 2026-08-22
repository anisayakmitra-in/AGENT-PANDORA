import unittest
from pathlib import Path

from scripts.validate_repo import validate_content, validate_paths


ROOT = Path(__file__).resolve().parents[1]


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
        private_key = (
            "-----BEGIN " + "PRIVATE KEY-----\nsecret\n-----END " + "PRIVATE KEY-----\n"
        )
        findings = validate_content(Path("signing.key"), private_key.encode("utf-8"))

        rendered = "\n".join(findings)
        self.assertTrue(any("private key" in finding for finding in findings))
        self.assertNotIn("secret", rendered)

    def test_rejects_mutable_github_action_reference(self) -> None:
        workflow = b"steps:\n  - uses: actions/checkout@v7\n"

        findings = validate_content(Path(".github/workflows/ci.yml"), workflow)

        self.assertTrue(any("full commit SHA" in finding for finding in findings))

    def test_accepts_pinned_github_action_reference(self) -> None:
        workflow = (
            b"steps:\n"
            b"  - uses: actions/checkout@"
            b"3d3c42e5aac5ba805825da76410c181273ba90b1\n"
        )

        self.assertEqual(
            validate_content(Path(".github/workflows/ci.yml"), workflow), []
        )

    def test_repository_defines_rust_codeql_analysis(self) -> None:
        workflow = ROOT / ".github" / "workflows" / "codeql.yml"

        self.assertTrue(workflow.is_file())
        content = workflow.read_text(encoding="utf-8")
        self.assertIn("languages: rust", content)
        self.assertIn("build-mode: none", content)


if __name__ == "__main__":
    unittest.main()
