from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


from scripts.release_scope import ReleaseScopeError, resolve_release_scope


ROOT = Path(__file__).resolve().parents[1]
VALIDATOR = ROOT / "scripts" / "release_scope.py"


class ReleaseScopeTests(unittest.TestCase):
    def make_root(self, scope: str = "cli-only", desktop_required: bool = False) -> Path:
        temporary = tempfile.TemporaryDirectory(prefix="pandora-release-scope-")
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        (root / "release-scope.json").write_text(
            json.dumps(
                {
                    "schema_version": 1,
                    "scope": scope,
                    "desktop_required": desktop_required,
                }
            )
            + "\n",
            encoding="utf-8",
        )
        return root

    def test_current_source_scope_allows_a_cli_only_beta(self) -> None:
        resolved = resolve_release_scope("v2.0.0-beta.8", ROOT)

        self.assertEqual(resolved["scope"], "cli-only")
        self.assertEqual(resolved["channel"], "beta")
        self.assertFalse(resolved["desktop_required"])

    def test_full_scope_is_valid_for_every_supported_channel(self) -> None:
        root = self.make_root("full", True)
        for tag, channel in (
            ("v2.0.0-alpha.1", "alpha"),
            ("v2.0.0-beta.1", "beta"),
            ("v2.0.0-rc.1", "release-candidate"),
            ("v2.0.0", "stable"),
        ):
            with self.subTest(tag=tag):
                resolved = resolve_release_scope(tag, root)
                self.assertEqual(resolved["scope"], "full")
                self.assertEqual(resolved["channel"], channel)
                self.assertTrue(resolved["desktop_required"])

    def test_cli_only_scope_rejects_release_candidate_and_stable(self) -> None:
        root = self.make_root("cli-only", False)
        for tag in ("v2.0.0-rc.1", "v2.0.0"):
            with self.subTest(tag=tag):
                with self.assertRaisesRegex(ReleaseScopeError, "requires full scope"):
                    resolve_release_scope(tag, root)

    def test_rejects_scope_and_desktop_requirement_disagreement(self) -> None:
        for scope, desktop_required in (("full", False), ("cli-only", True)):
            with self.subTest(scope=scope):
                root = self.make_root(scope, desktop_required)
                with self.assertRaisesRegex(ReleaseScopeError, "desktop_required"):
                    resolve_release_scope("v2.0.0-beta.1", root)

    def test_rejects_unknown_fields_and_malformed_values(self) -> None:
        cases = (
            {"schema_version": 2, "scope": "cli-only", "desktop_required": False},
            {"schema_version": 1, "scope": "other", "desktop_required": False},
            {"schema_version": 1, "scope": [], "desktop_required": False},
            {"schema_version": 1, "scope": "cli-only", "desktop_required": "false"},
            {
                "schema_version": 1,
                "scope": "cli-only",
                "desktop_required": False,
                "mutable_override": True,
            },
        )
        for document in cases:
            with self.subTest(document=document):
                root = self.make_root()
                (root / "release-scope.json").write_text(
                    json.dumps(document) + "\n", encoding="utf-8"
                )
                with self.assertRaises(ReleaseScopeError):
                    resolve_release_scope("v2.0.0-beta.1", root)

    def test_command_writes_only_validated_github_output(self) -> None:
        root = self.make_root("full", True)
        output = root / "github-output.txt"
        environment = {
            **os.environ,
            "PATH": f"C:\\Program Files\\Git\\cmd;{os.environ.get('PATH', '')}",
        }
        result = subprocess.run(
            [
                sys.executable,
                str(VALIDATOR),
                "v2.0.0-beta.1",
                "--root",
                str(root),
                "--github-output",
                str(output),
            ],
            check=False,
            capture_output=True,
            text=True,
            env=environment,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(
            output.read_text(encoding="utf-8"),
            "scope=full\nchannel=beta\ndesktop_required=true\n",
        )


if __name__ == "__main__":
    unittest.main()
