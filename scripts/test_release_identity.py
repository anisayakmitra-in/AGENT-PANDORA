import json
import re
import shutil
import subprocess
import sys
import unittest
import uuid
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
VALIDATOR = ROOT / "scripts" / "release_identity.py"


def make_temp_root() -> Path:
    match = re.search(
        r'(?m)^version = "([^"]+)"$', (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    )
    if not match:
        raise AssertionError("unable to read workspace version from Cargo.toml")
    root = ROOT / "scripts" / f"release-identity-{match.group(1)}-{uuid.uuid4().hex}"
    root.mkdir()
    return root


class ReleaseIdentityTests(unittest.TestCase):
    def run_validator(
        self,
        cargo_version: str,
        npm_version: str,
        tag: str,
        shell_version: str | None = None,
        powershell_version: str | None = None,
    ) -> subprocess.CompletedProcess[str]:
        root = make_temp_root()
        self.addCleanup(shutil.rmtree, root, True)
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
        scripts_dir = root / "scripts"
        scripts_dir.mkdir()
        (scripts_dir / "install.sh").write_text(
            f'version="${{PANDORA_VERSION:-v{shell_version or cargo_version}}}"\n',
            encoding="utf-8",
        )
        (scripts_dir / "install.ps1").write_text(
            f'$defaultVersion = "v{powershell_version or cargo_version}"\n',
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
            "2.0.0-beta.1", "2.0.0-beta.1", "v2.0.0-beta.1"
        )

        self.assertEqual(result.returncode, 0, result.stderr)

    def test_rejects_npm_version_drift(self) -> None:
        result = self.run_validator(
            "2.0.0-beta.1", "2.0.0-beta.0", "v2.0.0-beta.1"
        )

        self.assertEqual(result.returncode, 1)
        self.assertIn("npm package version", result.stdout)

    def test_rejects_tag_drift(self) -> None:
        result = self.run_validator(
            "2.0.0-beta.1", "2.0.0-beta.1", "v2.0.0-beta.0"
        )

        self.assertEqual(result.returncode, 1)
        self.assertIn("release tag", result.stdout)

    def test_rejects_installer_version_drift(self) -> None:
        for installer, versions in {
            "shell installer": ("2.0.0-beta.0", "2.0.0-beta.1"),
            "PowerShell installer": ("2.0.0-beta.1", "2.0.0-beta.0"),
        }.items():
            with self.subTest(installer=installer):
                result = self.run_validator(
                    "2.0.0-beta.1",
                    "2.0.0-beta.1",
                    "v2.0.0-beta.1",
                    *versions,
                )

                self.assertEqual(result.returncode, 1)
                self.assertIn(installer, result.stdout)


if __name__ == "__main__":
    unittest.main()
