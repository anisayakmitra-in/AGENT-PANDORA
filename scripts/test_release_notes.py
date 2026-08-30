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

    def test_release_workflow_verifies_and_publishes_release_evidence(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        verify = workflow.index("- name: Verify checksum signature")
        evidence = workflow.index("- name: Generate release evidence index")
        publish = workflow.index("- name: Publish GitHub release")
        self.assertLess(verify, evidence)
        self.assertLess(evidence, publish)
        self.assertIn("cosign verify-blob", workflow)
        self.assertIn('release_evidence.py "$GITHUB_REF_NAME"', workflow)
        self.assertIn("release-evidence.json", workflow)

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

    def test_release_workflow_verifies_desktop_signing_and_lifecycle_before_staging(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        build = workflow.index("- name: Build desktop bundle")
        macos_verify = workflow.index(
            "- name: Verify signed and notarized desktop bundle (macOS)"
        )
        windows_sign = workflow.index("- name: Sign desktop bundles (Windows)")
        lifecycle = workflow.index("- name: Verify desktop bundle lifecycle")
        stage = workflow.index("- name: Stage desktop artifacts (Unix)")

        self.assertLess(build, macos_verify)
        self.assertLess(build, windows_sign)
        self.assertLess(macos_verify, lifecycle)
        self.assertLess(windows_sign, lifecycle)
        self.assertLess(lifecycle, stage)
        self.assertIn("codesign --verify --deep --strict", workflow)
        self.assertIn("spctl --assess --type execute", workflow)
        self.assertIn("xcrun stapler validate", workflow)
        self.assertIn("signtool verify /pa /all /v", workflow)
        self.assertIn("npm run verify:bundle-lifecycle", workflow)
        self.assertIn("patchelf xvfb", workflow)
        self.assertIn("target/release/bundle", workflow)
        self.assertNotIn("apps/pandora-desktop/src-tauri/target/release/bundle", workflow)
        self.assertIn("Download verified native sidecar", workflow)
        self.assertIn("PANDORA_SIDECAR_SOURCE:", workflow)
        self.assertIn("PANDORA_DESKTOP_SOURCE_SIDECAR:", workflow)

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
        self.assertNotIn('PANDORA_INSTALL_DIR: ${{ runner.temp }}', workflow)
        self.assertIn(
            'export PANDORA_INSTALL_DIR="$RUNNER_TEMP/pandora-bin"', workflow
        )
        self.assertIn(
            '$env:PANDORA_INSTALL_DIR = Join-Path $env:RUNNER_TEMP "pandora-bin"',
            workflow,
        )
        self.assertIn('PANDORA_VERSION: ${{ github.ref_name }}', workflow)
        self.assertIn('expected="pandora ${GITHUB_REF_NAME#v}"', workflow)
        self.assertIn(
            '$expected = "pandora " + $env:GITHUB_REF_NAME.Substring(1)', workflow
        )

    def test_release_workflow_exercises_full_lifecycle(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn(
            'predecessor="$(python scripts/release_predecessor.py "$GITHUB_REF_NAME")"',
            workflow,
        )
        self.assertIn(
            "$predecessor = (& python scripts/release_predecessor.py "
            "$env:GITHUB_REF_NAME).Trim()",
            workflow,
        )
        for command in (
            "PANDORA_VERSION=\"$predecessor\" sh \"$installer\"",
            "PANDORA_VERSION=\"$GITHUB_REF_NAME\" sh \"$installer\"",
            '"$cli" update --artifact "$cli"',
            '"$cli" update --rollback',
            '"$cli" setup --json',
            '"$cli" doctor --json',
            '"$cli" backup create --output "$backup" --json',
            '"$cli" backup inspect --input "$backup" --json',
            '"$cli" backup restore --input "$backup" --yes --json',
            '"$cli" uninstall --dry-run --json',
            '"$cli" uninstall --yes --json',
            "$env:PANDORA_VERSION = $predecessor",
            "$env:PANDORA_VERSION = $env:GITHUB_REF_NAME",
            "update --artifact $cli",
            "update --rollback",
            "setup --json",
            "doctor --json",
            "backup create --output $backup --json",
            "backup inspect --input $backup --json",
            "backup restore --input $backup --yes --json",
            "uninstall --dry-run --json",
            "uninstall --yes --json",
        ):
            self.assertIn(command, workflow)
        self.assertIn("checksums.txt", workflow)
        self.assertIn("sentinel.txt", workflow)

    def test_release_workflow_smokes_published_desktop_packages(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("smoke-desktop:", workflow)
        self.assertIn(
            "Exercise published desktop package on ${{ matrix.platform }}", workflow
        )
        for bundle in (
            "desktop-linux-x64-*.deb",
            "desktop-macos-x64-*.dmg",
            "desktop-macos-arm64-*.dmg",
            "desktop-windows-x64-*.msi",
        ):
            self.assertIn(bundle, workflow)
        self.assertIn("gh release download", workflow)
        self.assertIn("verify_release_downloads.py", workflow)
        self.assertIn("PANDORA_DESKTOP_SOURCE_SIDECAR", workflow)
        self.assertIn("PANDORA_DESKTOP_BUNDLE_ROOT", workflow)
        self.assertIn("npm run verify:bundle-lifecycle", workflow)
