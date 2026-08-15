import hashlib
import json
import os
import platform
import re
import shutil
import subprocess
import unittest
import uuid
from pathlib import Path

from scripts.installer_contract import (
    artifact_name,
    expected_checksum,
    parse_checksums,
    release_url,
    cached_artifact,
    verify_checksum,
)


ROOT = Path(__file__).resolve().parent.parent


def workspace_temp_directory() -> Path:
    directory = ROOT / "scripts" / f"installer-test-{uuid.uuid4().hex}"
    directory.mkdir()
    return directory


class InstallerContractTests(unittest.TestCase):
    def test_selects_supported_native_artifacts(self) -> None:
        self.assertEqual(
            artifact_name("linux", "x86_64"),
            "pandora-x86_64-unknown-linux-gnu",
        )
        self.assertEqual(
            artifact_name("darwin", "arm64"),
            "pandora-aarch64-apple-darwin",
        )
        self.assertEqual(
            artifact_name("windows", "x86_64"),
            "pandora-x86_64-pc-windows-msvc.exe",
        )

    def test_rejects_unsupported_architecture(self) -> None:
        with self.assertRaises(ValueError):
            artifact_name("linux", "i686")

    def test_parses_and_verifies_exact_checksum(self) -> None:
        payload = b"pandora release"
        digest = hashlib.sha256(payload).hexdigest()
        manifest = parse_checksums(f"{digest}  pandora-linux\n")

        self.assertEqual(expected_checksum(manifest, "pandora-linux"), digest)
        self.assertTrue(verify_checksum(payload, digest))
        self.assertFalse(verify_checksum(payload + b"!", digest))

    def test_cached_artifact_requires_matching_checksum(self) -> None:
        payload = b"cached Pandora release"
        digest = hashlib.sha256(payload).hexdigest()
        directory = workspace_temp_directory()
        try:
            cache = directory
            artifact = cache / "pandora-linux"
            artifact.write_bytes(payload)
            self.assertEqual(cached_artifact(cache, artifact.name, digest), artifact)
            self.assertIsNone(cached_artifact(cache, artifact.name, "0" * 64))
        finally:
            shutil.rmtree(directory)

    def test_rejects_malformed_checksum_manifest(self) -> None:
        with self.assertRaises(ValueError):
            parse_checksums("not-a-digest  pandora-linux\n")

    def test_release_url_requires_https(self) -> None:
        self.assertEqual(
            release_url(
                "https://github.com/anisayakmitra-in/PANDORA-AGENT/releases/download",
                "v2.0.0-alpha.2",
                "pandora-linux",
            ),
            "https://github.com/anisayakmitra-in/PANDORA-AGENT/releases/download/v2.0.0-alpha.2/pandora-linux",
        )
        with self.assertRaises(ValueError):
            release_url("http://example.test/releases", "v2.0.0", "pandora-linux")

    def test_installers_require_checksum_verification(self) -> None:
        shell = (ROOT / "scripts" / "install.sh").read_text(encoding="utf-8")
        powershell = (ROOT / "scripts" / "install.ps1").read_text(encoding="utf-8")

        self.assertIn("checksums.txt", shell)
        self.assertIn("sha256sum", shell)
        self.assertIn("checksums.txt", powershell)
        self.assertIn("Get-FileHash", powershell)

    def test_installers_default_to_the_current_published_release(self) -> None:
        shell = (ROOT / "scripts" / "install.sh").read_text(encoding="utf-8")
        powershell = (ROOT / "scripts" / "install.ps1").read_text(encoding="utf-8")
        cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        version = re.search(r'^version = "([^"]+)"$', cargo, re.MULTILINE)
        self.assertIsNotNone(version)
        tag = f"v{version.group(1)}"

        self.assertIn(f'version="${{PANDORA_VERSION:-{tag}}}"', shell)
        self.assertIn(f'$defaultVersion = "{tag}"', powershell)

    def test_readme_pin_example_passes_version_to_the_installer_shell(self) -> None:
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        version = re.search(r'^version = "([^"]+)"$', cargo, re.MULTILINE)
        self.assertIsNotNone(version)
        tag = f"v{version.group(1)}"
        self.assertIn(
            f"curl -fsSL https://raw.githubusercontent.com/anisayakmitra-in/PANDORA-AGENT/main/scripts/install.sh | PANDORA_VERSION={tag} sh",
            readme,
        )
        self.assertNotIn(
            f"PANDORA_VERSION={tag} curl -fsSL",
            readme,
        )

    def test_npm_launcher_uses_the_current_public_package_identity(self) -> None:
        package = json.loads(
            (ROOT / "npm" / "pandora-cli" / "package.json").read_text(
                encoding="utf-8"
            )
        )

        self.assertEqual(package["name"], "pandora-agent")
        self.assertEqual(package["bin"]["pandora"], "bin/pandora.js")
        self.assertNotIn("o-pandora", package["name"])

    def test_launcher_rejects_tampered_offline_cache(self) -> None:
        launcher = ROOT / "npm" / "pandora-cli" / "bin" / "pandora.js"
        directory = workspace_temp_directory()
        try:
            cache = directory / "cache"
            platform_name = platform.system().lower()
            machine = platform.machine().lower()
            architecture = "x86_64" if machine in {"amd64", "x86_64"} else "arm64"
            artifact_name_for_host = artifact_name(platform_name, architecture)
            artifact = cache / "v2.0.0-alpha.2" / artifact_name_for_host
            artifact.parent.mkdir(parents=True)
            artifact.write_bytes(b"not a Pandora binary")
            marker = Path(f"{artifact}.sha256")
            marker.write_text("0" * 64 + "\n", encoding="utf-8")
            environment = os.environ.copy()
            environment.update(
                {
                    "PANDORA_OFFLINE": "1",
                    "PANDORA_CACHE_DIR": str(cache),
                    "PANDORA_VERSION": "v2.0.0-alpha.2",
                }
            )
            result = subprocess.run(
                ["node", str(launcher), "--version"],
                env=environment,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("checksum", result.stderr.lower())
        finally:
            shutil.rmtree(directory)

    def test_npm_launcher_replaces_stale_cache_file(self) -> None:
        test_script = ROOT / "scripts" / "test_npm_launcher.js"
        result = subprocess.run(
            ["node", str(test_script)],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
