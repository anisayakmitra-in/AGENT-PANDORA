from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from scripts.generate_release_manifests import (
    ManifestGenerationError,
    generate_homebrew_formula,
    generate_release_manifests,
    generate_scoop_manifest,
    generate_winget_installer_manifest,
    generate_winget_version_manifest,
    main,
    parse_checksum_file,
    validate_release_checksums,
)

SAMPLE_CHECKSUMS = {
    "pandora-aarch64-apple-darwin": "1111111111111111111111111111111111111111111111111111111111111111",
    "pandora-x86_64-apple-darwin": "2222222222222222222222222222222222222222222222222222222222222222",
    "pandora-x86_64-unknown-linux-gnu": "3333333333333333333333333333333333333333333333333333333333333333",
    "pandora-x86_64-pc-windows-msvc.exe": "4444444444444444444444444444444444444444444444444444444444444444",
}


class GenerateReleaseManifestsTests(unittest.TestCase):
    def _create_checksum_file(self, root: Path, entries: dict[str, str] | None = None) -> Path:
        data = entries if entries is not None else SAMPLE_CHECKSUMS
        checksums_path = root / "checksums.txt"
        lines = [f"{digest}  {name}" for name, digest in data.items()]
        checksums_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
        return checksums_path

    def test_parse_checksum_file_valid(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            path = self._create_checksum_file(Path(td))
            parsed = parse_checksum_file(path)
            self.assertEqual(len(parsed), 4)
            self.assertEqual(
                parsed["pandora-aarch64-apple-darwin"],
                "1111111111111111111111111111111111111111111111111111111111111111",
            )

    def test_parse_checksum_file_invalid_digest(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            bad_path = root / "checksums.txt"
            bad_path.write_text("invalid_hash  pandora-binary\n", encoding="utf-8")
            with self.assertRaises(ManifestGenerationError):
                parse_checksum_file(bad_path)

    def test_validate_release_checksums_missing_required(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            partial = {"pandora-aarch64-apple-darwin": "1" * 64}
            path = self._create_checksum_file(root, partial)
            with self.assertRaises(ManifestGenerationError):
                validate_release_checksums(path, require_all_targets=True)

    def test_generate_homebrew_formula(self) -> None:
        content = generate_homebrew_formula("2.0.0", "v2.0.0", SAMPLE_CHECKSUMS)
        self.assertIn("class Pandora < Formula", content)
        self.assertIn('version "2.0.0"', content)
        self.assertIn(SAMPLE_CHECKSUMS["pandora-aarch64-apple-darwin"], content)
        self.assertIn(SAMPLE_CHECKSUMS["pandora-x86_64-apple-darwin"], content)
        self.assertIn(SAMPLE_CHECKSUMS["pandora-x86_64-unknown-linux-gnu"], content)
        self.assertIn("def install", content)
        self.assertIn("test do", content)

    def test_generate_scoop_manifest(self) -> None:
        data = generate_scoop_manifest("2.0.0", "v2.0.0", SAMPLE_CHECKSUMS)
        self.assertEqual(data["version"], "2.0.0")
        self.assertEqual(data["architecture"]["64bit"]["hash"], SAMPLE_CHECKSUMS["pandora-x86_64-pc-windows-msvc.exe"])
        self.assertIn("pandora-x86_64-pc-windows-msvc.exe", data["architecture"]["64bit"]["url"])

    def test_generate_winget_manifests(self) -> None:
        ver = generate_winget_version_manifest("2.0.0")
        self.assertIn("PackageIdentifier: AnisayakmitraIn.AGENT-PANDORA", ver)
        self.assertIn("PackageVersion: 2.0.0", ver)
        self.assertIn("ManifestType: version", ver)

        inst = generate_winget_installer_manifest("2.0.0", "v2.0.0", SAMPLE_CHECKSUMS)
        self.assertIn("PackageIdentifier: AnisayakmitraIn.AGENT-PANDORA", inst)
        self.assertIn("PackageVersion: 2.0.0", inst)
        self.assertIn("Architecture: x64", inst)
        self.assertIn(SAMPLE_CHECKSUMS["pandora-x86_64-pc-windows-msvc.exe"], inst)
        self.assertIn("ManifestType: installer", inst)

    def test_generate_release_manifests_end_to_end(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            checksums_file = self._create_checksum_file(root)
            out_dir = root / "out"

            results = generate_release_manifests(
                output_dir=out_dir,
                checksums_path=checksums_file,
                tag="v2.0.0-beta.8",
                validate=True,
            )

            self.assertTrue(results["homebrew_formula"].is_file())
            self.assertTrue(results["scoop_manifest"].is_file())
            self.assertTrue(results["winget_version"].is_file())
            self.assertTrue(results["winget_installer"].is_file())

            # Check json parsing of scoop manifest
            parsed_scoop = json.loads(results["scoop_manifest"].read_text(encoding="utf-8"))
            self.assertEqual(parsed_scoop["version"], "2.0.0-beta.8")

    def test_cli_main_success(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            checksums_file = self._create_checksum_file(root)
            out_dir = root / "out"

            ret = main([
                "v2.0.0-beta.8",
                "--checksums",
                str(checksums_file),
                "--output-dir",
                str(out_dir),
            ])
            self.assertEqual(ret, 0)
            self.assertTrue((out_dir / "Formula" / "pandora.rb").is_file())
            self.assertTrue((out_dir / "bucket" / "pandora.json").is_file())
            self.assertTrue((out_dir / "winget" / "manifest.version.yaml").is_file())
            self.assertTrue((out_dir / "winget" / "manifest.installer.yaml").is_file())


if __name__ == "__main__":
    unittest.main()
