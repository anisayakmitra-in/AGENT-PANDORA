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
    def make_root(self, document: dict[str, object] | None = None) -> Path:
        temporary = tempfile.TemporaryDirectory(prefix="pandora-release-scope-")
        self.addCleanup(temporary.cleanup)
        root = Path(temporary.name)
        payload = {"schema_version": 2, "scope": "cli-only"}
        if document is not None:
            payload = document
        (root / "release-scope.json").write_text(
            json.dumps(payload) + "\n", encoding="utf-8"
        )
        return root

    def test_current_source_scope_allows_a_cli_only_beta(self) -> None:
        resolved = resolve_release_scope("v2.0.0-beta.8", ROOT)

        self.assertEqual(resolved["scope"], "cli-only")
        self.assertEqual(resolved["channel"], "beta")
        self.assertEqual(resolved["release_tag"], "v2.0.0-beta.8")
        self.assertNotIn("desktop_required", resolved)

    def test_the_desktop_tree_carries_no_desktop_requirement(self) -> None:
        """The removed second scope must not survive anywhere in the policy."""
        policy = json.loads((ROOT / "release-scope.json").read_text(encoding="utf-8"))

        self.assertEqual(policy, {"schema_version": 2, "scope": "cli-only"})
        self.assertNotIn("desktop_required", policy)

    def test_cli_only_scope_allows_every_supported_channel(self) -> None:
        for tag, channel in (
            ("v2.0.0-alpha.1", "alpha"),
            ("v2.0.0-beta.1", "beta"),
            ("v2.0.0-rc.1", "release-candidate"),
            ("v2.0.0", "stable"),
        ):
            with self.subTest(tag=tag):
                resolved = resolve_release_scope(tag, self.make_root())
                self.assertEqual(resolved["scope"], "cli-only")
                self.assertEqual(resolved["channel"], channel)

    def test_rejects_the_removed_full_scope(self) -> None:
        """`scope: full` selected a desktop that no longer exists."""
        root = self.make_root({"schema_version": 2, "scope": "full"})
        with self.assertRaisesRegex(ReleaseScopeError, "unsupported"):
            resolve_release_scope("v2.0.0-beta.1", root)

    def test_rejects_a_policy_still_carrying_the_desktop_flag(self) -> None:
        root = self.make_root(
            {"schema_version": 2, "scope": "cli-only", "desktop_required": False}
        )
        with self.assertRaisesRegex(ReleaseScopeError, "unsupported shape"):
            resolve_release_scope("v2.0.0-beta.1", root)

    def test_rejects_unknown_fields_and_malformed_values(self) -> None:
        cases = (
            {"schema_version": 1, "scope": "cli-only"},
            {"schema_version": 2, "scope": "other"},
            {"schema_version": 2, "scope": []},
            {"schema_version": 2, "scope": "cli-only", "mutable_override": True},
            {"scope": "cli-only"},
        )
        for document in cases:
            with self.subTest(document=document):
                root = self.make_root(document)
                with self.assertRaises(ReleaseScopeError):
                    resolve_release_scope("v2.0.0-beta.1", root)

    def test_rejects_an_invalid_release_tag(self) -> None:
        for tag in ("2.0.0", "latest", "v2.0"):
            with self.subTest(tag=tag):
                with self.assertRaisesRegex(ReleaseScopeError, "invalid release tag"):
                    resolve_release_scope(tag, self.make_root())

    def test_command_writes_only_validated_github_output(self) -> None:
        root = self.make_root()
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
            "scope=cli-only\nchannel=beta\n",
        )


if __name__ == "__main__":
    unittest.main()