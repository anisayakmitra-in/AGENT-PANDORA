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


if __name__ == "__main__":
    unittest.main()
