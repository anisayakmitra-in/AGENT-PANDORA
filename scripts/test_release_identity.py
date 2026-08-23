import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
VALIDATOR = ROOT / "scripts" / "release_identity.py"


class ReleaseIdentityTests(unittest.TestCase):
    def run_validator(
        self, cargo_version: str, npm_version: str, tag: str
    ) -> subprocess.CompletedProcess[str]:
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        (root / "Cargo.toml").write_text(
            f'[workspace.package]\nversion = "{cargo_version}"\n',
            encoding="utf-8",
        )
        package_dir = root / "npm" / "pandora-cli"
        package_dir.mkdir(parents=True)
        (package_dir / "package.json").write_text(
            json.dumps({"version": npm_version}),
            encoding="utf-8",
        )
        return subprocess.run(
            [sys.executable, str(VALIDATOR), "--root", str(root), tag],
            capture_output=True,
            text=True,
            check=False,
        )

    def test_matching_workspace_package_and_tag_pass(self) -> None:
        result = self.run_validator(
            "2.0.0-alpha.7", "2.0.0-alpha.7", "v2.0.0-alpha.7"
        )

        self.assertEqual(result.returncode, 0, result.stderr)

    def test_rejects_npm_version_drift(self) -> None:
        result = self.run_validator(
            "2.0.0-alpha.7", "2.0.0-alpha.6", "v2.0.0-alpha.7"
        )

        self.assertEqual(result.returncode, 1)
        self.assertIn("npm package version", result.stdout)

    def test_rejects_tag_drift(self) -> None:
        result = self.run_validator(
            "2.0.0-alpha.7", "2.0.0-alpha.7", "v2.0.0-alpha.6"
        )

        self.assertEqual(result.returncode, 1)
        self.assertIn("release tag", result.stdout)


if __name__ == "__main__":
    unittest.main()
