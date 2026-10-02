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


def posix_shell_command() -> str | None:
    if platform.system() == "Windows":
        for program_files in (
            os.environ.get("ProgramFiles"),
            os.environ.get("ProgramW6432"),
            os.environ.get("ProgramFiles(x86)"),
        ):
            if not program_files:
                continue
            for relative_path in (
                ("Git", "bin", "bash.exe"),
                ("Git", "usr", "bin", "sh.exe"),
            ):
                candidate = Path(program_files, *relative_path)
                if candidate.is_file():
                    return str(candidate)
        return None
    return shutil.which("sh") or shutil.which("bash")


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

    def test_accepts_historic_dist_prefixed_checksum_entries(self) -> None:
        payload = b"historic Pandora release"
        digest = hashlib.sha256(payload).hexdigest()
        manifest = parse_checksums(f"{digest}  dist/pandora-linux\n")

        self.assertEqual(expected_checksum(manifest, "pandora-linux"), digest)

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
            shutil.rmtree(directory, ignore_errors=True)

    def test_rejects_malformed_checksum_manifest(self) -> None:
        with self.assertRaises(ValueError):
            parse_checksums("not-a-digest  pandora-linux\n")

    def test_release_url_requires_https(self) -> None:
        self.assertEqual(
            release_url(
                "https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download",
                "v2.0.0-beta.1",
                "pandora-linux",
            ),
            "https://github.com/anisayakmitra-in/AGENT-PANDORA/releases/download/v2.0.0-beta.1/pandora-linux",
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

    def test_posix_installer_rejects_malformed_version_components(self) -> None:
        shell = (ROOT / "scripts" / "install.sh").read_text(encoding="utf-8")
        self.assertIn(
            "grep -Eq '^v[0-9]+\\.[0-9]+\\.[0-9]+(-[0-9A-Za-z.-]+)?$'",
            shell,
        )

        shell_command = posix_shell_command()
        if shell_command is None:
            self.skipTest("a POSIX shell is required to execute install.sh")

        environment = os.environ.copy()
        environment.update(
            {
                "PANDORA_VERSION": "v2x.0.0",
                "PANDORA_RELEASE_BASE_URL": "https://example.invalid/releases/download",
            }
        )
        result = subprocess.run(
            [shell_command, str(ROOT / "scripts" / "install.sh")],
            env=environment,
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(
            "must be a SemVer tag" in result.stderr
            or "couldn't create signal pipe" in result.stderr
        )

    def test_installers_default_to_verifying_the_release_signature(self) -> None:
        """The checksum manifest is signed; verifying it must be the default.

        The audit finding behind this is that both installers only verified the
        manifest signature when `PANDORA_REQUIRE_SIGNATURE=1` was set
        explicitly, so the shipped default proved an artifact matched a manifest
        nobody had authenticated. The release workflow already publishes
        `checksums.txt.sig` and `checksums.txt.pem`, so the verification code was
        present and correct; only the default was wrong.
        """
        shell = (ROOT / "scripts" / "install.sh").read_text(encoding="utf-8")
        powershell = (ROOT / "scripts" / "install.ps1").read_text(encoding="utf-8")

        # The default must be "verify", expressed as "skip only on an explicit 0".
        self.assertIn('PANDORA_REQUIRE_SIGNATURE:-1', shell)
        self.assertIn('!= "0"', shell)
        self.assertNotIn('PANDORA_REQUIRE_SIGNATURE:-0', shell)
        self.assertIn('PANDORA_REQUIRE_SIGNATURE -ne "0"', powershell)
        self.assertNotIn('PANDORA_REQUIRE_SIGNATURE -eq "1"', powershell)

        # Both must still reach the real verification, not just flip a flag.
        for script, marker in ((shell, "cosign verify-blob"), (powershell, "cosign verify-blob")):
            self.assertIn(marker, script)
            self.assertIn("checksums.txt.sig", script)
            self.assertIn("certificate-identity", script)

        # A missing identity must fail rather than silently skip.
        self.assertIn("PANDORA_COSIGN_IDENTITY is required", shell)
        self.assertIn("PANDORA_COSIGN_IDENTITY is required", powershell)

    def test_the_release_smoke_install_verifies_the_signature_it_publishes(self) -> None:
        """The pipeline must not install its own artifact on an unverified manifest.

        This was the gap that made the shipped evidence weaker than it looked: the
        smoke-install job downloaded the published installer and relied on the old
        insecure default, so a manifest-signing regression would not have failed
        the release.
        """
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )
        start = workflow.index("  smoke-install:")
        job = workflow[start:]
        # Bound the job slice at the next top-level job key so a match elsewhere in
        # the workflow cannot satisfy this assertion.
        next_key = re.search(r"\n  [a-z][a-z0-9-]*:\n", job)
        job = job[: next_key.start()] if next_key else job

        self.assertIn('PANDORA_REQUIRE_SIGNATURE: "1"', job)
        self.assertIn("PANDORA_COSIGN_IDENTITY:", job)
        self.assertIn("sigstore/cosign-installer@", job)
        self.assertNotIn('PANDORA_REQUIRE_SIGNATURE: "0"', job)

    def test_readme_pin_example_passes_version_to_the_installer_shell(self) -> None:
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        version = re.search(r'^version = "([^"]+)"$', cargo, re.MULTILINE)
        self.assertIsNotNone(version)
        tag = f"v{version.group(1)}"
        self.assertIn(
            f"curl -fsSL https://raw.githubusercontent.com/anisayakmitra-in/AGENT-PANDORA/main/scripts/install.sh | PANDORA_VERSION={tag} sh",
            readme,
        )
        self.assertNotIn(
            f"PANDORA_VERSION={tag} curl -fsSL",
            readme,
        )

    def test_npm_launcher_uses_the_current_package_identity(self) -> None:
        package = json.loads(
            (ROOT / "npm" / "pandora-cli" / "package.json").read_text(
                encoding="utf-8"
            )
        )

        self.assertEqual(package["name"], "pandora-agent")
        self.assertEqual(package["bin"]["pandora"], "bin/pandora.js")
        self.assertNotIn("o-pandora", package["name"])

    def test_platform_docs_mark_npm_registry_distribution_unavailable(self) -> None:
        platforms = (ROOT / "docs" / "PLATFORMS.md").read_text(encoding="utf-8")
        changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")

        self.assertIn("not published to the public npm registry", platforms)
        self.assertNotIn("A public `pandora-agent` npm/Bun launcher", changelog)

    def test_launcher_rejects_tampered_offline_cache(self) -> None:
        launcher = ROOT / "npm" / "pandora-cli" / "bin" / "pandora.js"
        directory = workspace_temp_directory()
        try:
            cache = directory / "cache"
            platform_name = platform.system().lower()
            machine = platform.machine().lower()
            architecture = "x86_64" if machine in {"amd64", "x86_64"} else "arm64"
            artifact_name_for_host = artifact_name(platform_name, architecture)
            artifact = cache / "v2.0.0-beta.1" / artifact_name_for_host
            artifact.parent.mkdir(parents=True)
            artifact.write_bytes(b"not a Pandora binary")
            marker = Path(f"{artifact}.sha256")
            marker.write_text("0" * 64 + "\n", encoding="utf-8")
            environment = os.environ.copy()
            environment.update(
                {
                    "PANDORA_OFFLINE": "1",
                    "PANDORA_CACHE_DIR": str(cache),
                    "PANDORA_VERSION": "v2.0.0-beta.1",
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
            shutil.rmtree(directory, ignore_errors=True)

    def test_npm_launcher_normalizes_package_version_to_a_release_tag(self) -> None:
        launcher = ROOT / "npm" / "pandora-cli" / "bin" / "pandora.js"
        directory = workspace_temp_directory()
        try:
            cache = directory / "cache"
            platform_name = platform.system().lower()
            machine = platform.machine().lower()
            architecture = "x86_64" if machine in {"amd64", "x86_64"} else "arm64"
            artifact_name_for_host = artifact_name(platform_name, architecture)
            artifact = cache / "v2.0.0-beta.1" / artifact_name_for_host
            artifact.parent.mkdir(parents=True)
            payload = b"not an executable, but it is checksum-valid"
            artifact.write_bytes(payload)
            marker = Path(f"{artifact}.sha256")
            marker.write_text(hashlib.sha256(payload).hexdigest() + "\n", encoding="utf-8")
            environment = os.environ.copy()
            environment.pop("PANDORA_VERSION", None)
            environment.update(
                {
                    "PANDORA_OFFLINE": "1",
                    "PANDORA_CACHE_DIR": str(cache),
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
            self.assertNotIn("semver", result.stderr.lower())
            self.assertNotIn("checksum", result.stderr.lower())
        finally:
            shutil.rmtree(directory, ignore_errors=True)

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
