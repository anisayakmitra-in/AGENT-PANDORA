from __future__ import annotations

import json
import subprocess
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
NEXT_VERSION = "2.0.0-beta.8"
NEXT_TAG = f"v{NEXT_VERSION}"
CONSUMED_TAG = "v2.0.0-beta.7"


class NextBetaIdentityTests(unittest.TestCase):
    def test_runtime_and_package_surfaces_use_the_next_beta(self) -> None:
        cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
        npm = json.loads(
            (ROOT / "npm" / "pandora-cli" / "package.json").read_text(encoding="utf-8")
        )

        self.assertIn(f'version = "{NEXT_VERSION}"', cargo)
        self.assertEqual(npm["version"], NEXT_VERSION)

    def test_installers_and_sdk_manifests_use_the_next_beta(self) -> None:
        shell = (ROOT / "scripts" / "install.sh").read_text(encoding="utf-8")
        powershell = (ROOT / "scripts" / "install.ps1").read_text(encoding="utf-8")
        self.assertIn(f"PANDORA_VERSION:-v{NEXT_VERSION}", shell)
        self.assertIn(f'$defaultVersion = "v{NEXT_VERSION}"', powershell)

        manifests = sorted(ROOT.glob("sdk/**/pandora.package.json"))
        self.assertEqual(len(manifests), 6)
        for manifest_path in manifests:
            with self.subTest(manifest=manifest_path.relative_to(ROOT)):
                document = json.loads(manifest_path.read_text(encoding="utf-8"))
                self.assertEqual(
                    document["compatibility"]["runtime"],
                    f"pandora={NEXT_VERSION}",
                )

        for fixture in (
            ROOT / "fuzz" / "corpus" / "manifest_parser" / "valid.json",
            ROOT / "sdk" / "gene-pack" / "fixtures" / "negative" / "undeclared-capability.json",
            ROOT / "sdk" / "gene-pack" / "fixtures" / "negative" / "traversal-id.json",
        ):
            with self.subTest(fixture=fixture.relative_to(ROOT)):
                document = json.loads(fixture.read_text(encoding="utf-8"))
                self.assertEqual(
                    document["compatibility"]["runtime"],
                    f"pandora={NEXT_VERSION}",
                )

    def test_release_identity_accepts_next_and_rejects_consumed_tag(self) -> None:
        validator = ROOT / "scripts" / "release_identity.py"
        accepted = subprocess.run(
            [sys.executable, str(validator), "--root", str(ROOT), NEXT_TAG],
            capture_output=True,
            text=True,
            check=False,
        )
        rejected = subprocess.run(
            [sys.executable, str(validator), "--root", str(ROOT), CONSUMED_TAG],
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertEqual(accepted.returncode, 0, accepted.stdout + accepted.stderr)
        self.assertNotEqual(rejected.returncode, 0)
        self.assertIn("does not match", rejected.stdout)

    def test_changelog_and_release_policy_name_the_next_beta(self) -> None:
        changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
        releases = (ROOT / "RELEASES.md").read_text(encoding="utf-8")
        readme = (ROOT / "README.md").read_text(encoding="utf-8")

        self.assertIn("## v2.0.0-beta.8", changelog)
        self.assertIn("## v2.0.0-beta.8", releases)
        self.assertIn("next prerelease identity is `2.0.0-beta.8`", readme)


if __name__ == "__main__":
    unittest.main()
