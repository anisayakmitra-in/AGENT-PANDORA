import unittest
from pathlib import Path

from scripts.release_notes import extract_release_notes


ROOT = Path(__file__).resolve().parent.parent


class ReleaseNotesTests(unittest.TestCase):
    def test_extracts_only_the_requested_tag_section(self) -> None:
        changelog = """# Changelog

## Unreleased

- Not published.

## v2.0.0-alpha.2

Published notes.

### Shipped

- One feature.

## v2.0.0-alpha.1

Older notes.
"""

        self.assertEqual(
            extract_release_notes(changelog, "v2.0.0-alpha.2"),
            "Published notes.\n\n### Shipped\n\n- One feature.",
        )

    def test_rejects_a_missing_tag(self) -> None:
        with self.assertRaisesRegex(ValueError, "missing"):
            extract_release_notes("# Changelog\n", "v2.0.0-alpha.2")

    def test_rejects_an_empty_tag_section(self) -> None:
        with self.assertRaisesRegex(ValueError, "empty"):
            extract_release_notes("# Changelog\n\n## v2.0.0-alpha.2\n", "v2.0.0-alpha.2")

    def test_release_workflow_publishes_tag_scoped_notes(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn(
            'python scripts/release_notes.py "$GITHUB_REF_NAME" > "$RUNNER_TEMP/release-notes.md"',
            workflow,
        )
        self.assertIn("body_path: ${{ runner.temp }}/release-notes.md", workflow)
        self.assertNotIn("body_path: CHANGELOG.md", workflow)

    def test_release_workflow_smokes_native_cli_before_uploading_assets(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        smoke_step = workflow.index("- name: Smoke test native CLI")
        unix_stage = workflow.index("- name: Stage Unix artifact")
        windows_stage = workflow.index("- name: Stage Windows artifact")

        self.assertLess(smoke_step, unix_stage)
        self.assertLess(smoke_step, windows_stage)
        self.assertIn('expected="pandora ${GITHUB_REF_NAME#v}"', workflow)
        self.assertIn(
            '$expected = "pandora " + $env:GITHUB_REF_NAME.Substring(1)', workflow
        )

    def test_release_workflow_smokes_published_installers_on_fresh_runners(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("smoke-install:", workflow)
        self.assertIn("needs: publish", workflow)
        for operating_system in (
            "ubuntu-latest",
            "macos-15-intel",
            "macos-14",
            "windows-latest",
        ):
            self.assertIn(f"- os: {operating_system}", workflow)
        self.assertIn("releases/download/${GITHUB_REF_NAME}/install.sh", workflow)
        self.assertIn("releases/download/$env:GITHUB_REF_NAME/install.ps1", workflow)
        self.assertIn('PANDORA_INSTALL_DIR: ${{ runner.temp }}/pandora-bin', workflow)
        self.assertIn('PANDORA_VERSION: ${{ github.ref_name }}', workflow)
        self.assertIn('expected="pandora ${GITHUB_REF_NAME#v}"', workflow)
        self.assertIn(
            '$expected = "pandora " + $env:GITHUB_REF_NAME.Substring(1)', workflow
        )
