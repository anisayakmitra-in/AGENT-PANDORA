from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
VALIDATOR = ROOT / "scripts" / "release_identity.py"


def make_temp_root() -> Path:
    root = Path(tempfile.mkdtemp(prefix="pandora-release-identity-"))
    (root / "npm" / "pandora-cli").mkdir(parents=True)
    (root / "scripts").mkdir(parents=True)
    return root


class ReleaseIdentityTests(unittest.TestCase):
    def run_validator(
        self,
        cargo_version: str,
        npm_version: str,
        tag: str,
        *,
        shell_version: str | None = None,
        powershell_version: str | None = None,
    ) -> subprocess.CompletedProcess[str]:
        root = make_temp_root()
        self.addCleanup(shutil.rmtree, root, True)
        (root / "Cargo.toml").write_text(
            f'[workspace.package]\nversion = "{cargo_version}"\n',
            encoding="utf-8",
        )
        (root / "npm" / "pandora-cli" / "package.json").write_text(
            json.dumps({"name": "pandora-agent", "version": npm_version}) + "\n",
            encoding="utf-8",
        )
        (root / "scripts" / "install.sh").write_text(
            f'version="${{PANDORA_VERSION:-v{shell_version or cargo_version}}}"\n',
            encoding="utf-8",
        )
        (root / "scripts" / "install.ps1").write_text(
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
        result = self.run_validator("2.0.0-beta.1", "2.0.0-beta.1", "v2.0.0-beta.1")

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("release identity v2.0.0-beta.1 verified", result.stdout)

    def test_rejects_npm_version_drift(self) -> None:
        result = self.run_validator("2.0.0-beta.1", "2.0.0-beta.2", "v2.0.0-beta.1")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match", result.stdout)

    def test_rejects_tag_drift(self) -> None:
        result = self.run_validator("2.0.0-beta.1", "2.0.0-beta.1", "v2.0.0-beta.9")

        self.assertNotEqual(result.returncode, 0)
        self.assertIn("does not match", result.stdout)

    def test_rejects_installer_version_drift(self) -> None:
        for keyword in ("shell", "powershell"):
            with self.subTest(installer=keyword):
                kwargs = {
                    "shell_version": "2.0.0-beta.9",
                    "powershell_version": "2.0.0-beta.9",
                }
                result = self.run_validator(
                    "2.0.0-beta.1", "2.0.0-beta.1", "v2.0.0-beta.1", **kwargs
                )
                self.assertNotEqual(result.returncode, 0)
                lowered = result.stdout.lower()
                self.assertTrue(
                    "shell" in lowered or "powershell" in lowered, result.stdout
                )

    def test_the_repository_passes_with_no_optional_flag(self) -> None:
        """The removed opt-in flag must not be needed, or required, to pass."""
        with (ROOT / "Cargo.toml").open("rb") as cargo_file:
            import tomllib

            version = tomllib.load(cargo_file)["workspace"]["package"]["version"]

        result = subprocess.run(
            [sys.executable, str(VALIDATOR), "--root", str(ROOT), f"v{version}"],
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()