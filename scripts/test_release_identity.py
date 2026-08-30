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
        desktop_npm_version: str | None = None,
        desktop_lock_version: str | None = None,
        desktop_cargo_version: str | None = None,
        tauri_version_source: str = "../package.json",
        windows_msi_version: str = "2.0.0.1",
        windows_upgrade_code: str = "43f9019a-cb48-59a1-b463-5508bd89d386",
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
        desktop_dir = root / "apps" / "pandora-desktop"
        desktop_dir.mkdir(parents=True)
        resolved_desktop_npm_version = desktop_npm_version or cargo_version
        resolved_desktop_lock_version = desktop_lock_version or cargo_version
        (desktop_dir / "package.json").write_text(
            json.dumps({"version": resolved_desktop_npm_version}),
            encoding="utf-8",
        )
        (desktop_dir / "package-lock.json").write_text(
            json.dumps(
                {
                    "version": resolved_desktop_lock_version,
                    "packages": {"": {"version": resolved_desktop_lock_version}},
                }
            ),
            encoding="utf-8",
        )
        desktop_tauri_dir = desktop_dir / "src-tauri"
        desktop_tauri_dir.mkdir()
        (desktop_tauri_dir / "Cargo.toml").write_text(
            "[package]\n"
            f'version = "{desktop_cargo_version or cargo_version}"\n',
            encoding="utf-8",
        )
        (desktop_tauri_dir / "tauri.conf.json").write_text(
            json.dumps(
                {
                    "version": tauri_version_source,
                    "bundle": {
                        "windows": {
                            "wix": {
                                "version": windows_msi_version,
                                "upgradeCode": windows_upgrade_code,
                            }
                        }
                    },
                }
            ),
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

    def test_stable_release_uses_three_field_msi_version(self) -> None:
        result = self.run_validator(
            "2.0.0",
            "2.0.0",
            "v2.0.0",
            windows_msi_version="2.0.0",
        )

        self.assertEqual(result.returncode, 0, result.stdout)

    def test_prerelease_requires_numeric_msi_build_identity(self) -> None:
        result = self.run_validator(
            "2.0.0-beta",
            "2.0.0-beta",
            "v2.0.0-beta",
        )

        self.assertEqual(result.returncode, 1)
        self.assertIn("must end in a numeric identifier", result.stdout)

    def test_rejects_npm_version_drift(self) -> None:
        result = self.run_validator(
            "2.0.0-beta.1", "2.0.0-beta.0", "v2.0.0-beta.1"
        )

        self.assertEqual(result.returncode, 1)
        self.assertIn("npm package version", result.stdout)

    def test_rejects_desktop_version_drift(self) -> None:
        cases = {
            "desktop npm package version": {
                "desktop_npm_version": "2.0.0-beta.0"
            },
            "desktop package lock versions": {
                "desktop_lock_version": "2.0.0-beta.0"
            },
            "desktop Cargo package version": {
                "desktop_cargo_version": "2.0.0-beta.0"
            },
            "Tauri version must resolve": {
                "tauri_version_source": "2.0.0-beta.1"
            },
            "desktop Windows MSI version": {
                "windows_msi_version": "2.0.0.2"
            },
            "desktop Windows MSI upgrade code changed": {
                "windows_upgrade_code": "00000000-0000-0000-0000-000000000000"
            },
        }
        for message, overrides in cases.items():
            with self.subTest(message=message):
                result = self.run_validator(
                    "2.0.0-beta.1",
                    "2.0.0-beta.1",
                    "v2.0.0-beta.1",
                    **overrides,
                )

                self.assertEqual(result.returncode, 1)
                self.assertIn(message, result.stdout)

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
