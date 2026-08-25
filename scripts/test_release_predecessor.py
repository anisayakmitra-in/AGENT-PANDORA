from pathlib import Path
import subprocess
import sys
import unittest
import shutil
import uuid

from scripts.release_predecessor import ReleasePredecessorError, find_predecessor

ROOT = Path(__file__).resolve().parents[1]


def make_temp_root() -> Path:
    root = ROOT / "scripts" / f"release-predecessor-{uuid.uuid4().hex}"
    root.mkdir()
    return root


class ReleasePredecessorTests(unittest.TestCase):
    def test_finds_the_immediately_previous_release_section(self) -> None:
        changelog = """# Changelog

## Unreleased

## v2.0.0-beta.1

Current beta.

## v2.0.0-alpha.6

Previous alpha.

## v2.0.0-alpha.5

Older alpha.
"""

        self.assertEqual(
            find_predecessor(changelog, "v2.0.0-beta.1"),
            "v2.0.0-alpha.6",
        )

    def test_follows_semver_prerelease_precedence(self) -> None:
        changelog = """# Changelog

## v2.0.0-rc.1

Release candidate.

## v2.0.0-beta.11

Previous beta.
"""

        self.assertEqual(
            find_predecessor(changelog, "v2.0.0-rc.1"),
            "v2.0.0-beta.11",
        )

    def test_rejects_duplicate_release_sections(self) -> None:
        changelog = """# Changelog

## v2.0.0-beta.1

First.

## v2.0.0-alpha.6

Previous.

## v2.0.0-beta.1

Duplicate.
"""

        with self.assertRaisesRegex(
            ReleasePredecessorError,
            "duplicate release section",
        ):
            find_predecessor(changelog, "v2.0.0-beta.1")

    def test_rejects_a_missing_current_release(self) -> None:
        changelog = """# Changelog

## v2.0.0-alpha.6

Only release.
"""

        with self.assertRaisesRegex(
            ReleasePredecessorError,
            "current release section is missing",
        ):
            find_predecessor(changelog, "v2.0.0-beta.1")

    def test_rejects_malformed_release_headings(self) -> None:
        changelog = """# Changelog

## v2.0-beta.1

Malformed.

## v2.0.0-alpha.6

Previous.
"""

        with self.assertRaisesRegex(
            ReleasePredecessorError,
            "invalid release tag",
        ):
            find_predecessor(changelog, "v2.0.0-beta.1")

    def test_rejects_a_release_without_a_predecessor(self) -> None:
        changelog = """# Changelog

## v2.0.0-alpha.1

First release.
"""

        with self.assertRaisesRegex(
            ReleasePredecessorError,
            "predecessor release section is missing",
        ):
            find_predecessor(changelog, "v2.0.0-alpha.1")

    def test_rejects_a_predecessor_that_is_not_older(self) -> None:
        changelog = """# Changelog

## v2.0.0-beta.1

Current.

## v2.0.0-rc.1

Incorrectly ordered newer release.
"""

        with self.assertRaisesRegex(
            ReleasePredecessorError,
            "must be older than",
        ):
            find_predecessor(changelog, "v2.0.0-beta.1")

    def test_cli_prints_exactly_one_predecessor_tag(self) -> None:
        changelog = """# Changelog

## v2.0.0-beta.1

Current.

## v2.0.0-alpha.6

Previous.
"""
        directory = make_temp_root()
        self.addCleanup(shutil.rmtree, directory, True)
        path = directory / "CHANGELOG.md"
        path.write_text(changelog, encoding="utf-8")
        result = subprocess.run(
            [
                sys.executable,
                str(Path(__file__).with_name("release_predecessor.py")),
                "v2.0.0-beta.1",
                "--changelog",
                str(path),
            ],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, "v2.0.0-alpha.6\n")
        self.assertEqual(result.stderr, "")

    def test_cli_reports_a_bounded_error_without_a_traceback(self) -> None:
        directory = make_temp_root()
        self.addCleanup(shutil.rmtree, directory, True)
        path = directory / "CHANGELOG.md"
        path.write_text("# Changelog\n", encoding="utf-8")
        result = subprocess.run(
            [
                sys.executable,
                str(Path(__file__).with_name("release_predecessor.py")),
                "v2.0.0-beta.1",
                "--changelog",
                str(path),
            ],
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertIn("current release section is missing", result.stderr)
        self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
