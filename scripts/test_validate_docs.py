import unittest
from pathlib import Path

from scripts.validate_docs import validate_text


class ValidateDocumentationTests(unittest.TestCase):
    def test_accepts_clear_status_documentation(self) -> None:
        findings = validate_text(
            Path("docs/ARCHITECTURE.md"),
            "Status: Design-only\nThe authority chain is documented here.\n",
        )

        self.assertEqual(findings, [])

    def test_rejects_unfinished_documentation_markers(self) -> None:
        findings = validate_text(Path("docs/CLI.md"), "TODO: describe setup\n")

        self.assertEqual(findings, ["docs/CLI.md: contains unfinished documentation marker"])

    def test_rejects_missing_local_markdown_link(self) -> None:
        findings = validate_text(
            Path("README.md"),
            "[missing](docs/does-not-exist.md)\n",
        )

        self.assertEqual(
            findings,
            ["README.md: local link does not exist: docs/does-not-exist.md"],
        )

    def test_accepts_external_and_anchor_links(self) -> None:
        findings = validate_text(
            Path("README.md"),
            "[website](https://example.com) [section](#setup)\n",
        )

        self.assertEqual(findings, [])


if __name__ == "__main__":
    unittest.main()
