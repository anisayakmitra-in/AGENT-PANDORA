from __future__ import annotations

import hashlib
import json
import tempfile
import unittest
from pathlib import Path

from scripts.render_channel_manifests import (
    CHANNELS,
    ChannelManifestError,
    render_channel_manifests,
    render_template,
)

_ARTIFACTS = {
    "pandora-aarch64-apple-darwin": b"arm64 mac binary\n",
    "pandora-x86_64-apple-darwin": b"x64 mac binary\n",
    "pandora-x86_64-pc-windows-msvc.exe": b"windows binary\n",
}


class RenderChannelManifestsTests(unittest.TestCase):
    def _release(self, root: Path) -> tuple[Path, dict[str, str]]:
        """A release directory shaped like the publish job's dist/."""
        dist = root / "dist"
        dist.mkdir()
        digests = {}
        for name, payload in _ARTIFACTS.items():
            (dist / name).write_bytes(payload)
            digests[name] = hashlib.sha256(payload).hexdigest()
        manifest = dist / "checksums.txt"
        manifest.write_text(
            "".join(f"{digest}  {name}\n" for name, digest in sorted(digests.items())),
            encoding="utf-8",
        )
        return manifest, digests

    def test_renders_every_configured_channel(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, digests = self._release(root)

            rendered = render_channel_manifests(
                manifest, root / "out", tag="v2.0.0"
            )

            self.assertEqual(set(rendered), set(CHANNELS))
            for path in rendered.values():
                self.assertTrue(Path(path).is_file())

    def test_rendered_manifests_carry_the_real_checksums(self) -> None:
        """The SHA-256 must come from checksums.txt, not be hand-written.

        A channel manifest is only trustworthy if its digest is the one the
        release published. Each rendered file must contain the exact digest for
        the artifact that channel installs.
        """
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, digests = self._release(root)
            out = root / "out"

            render_channel_manifests(manifest, out, tag="v2.0.0")

            for channel, spec in CHANNELS.items():
                with self.subTest(channel=channel):
                    content = Path(out / spec["output"]).read_text(encoding="utf-8")
                    expected = digests[spec["artifact"]]
                    self.assertIn(expected, content)
                    # None of this renderer's own placeholders may survive.
                    for placeholder in (
                        "VERSION",
                        "SHA256",
                        "ARTIFACT",
                        "ARCH",
                        "CHANNEL",
                    ):
                        self.assertNotIn(f"{{{{ {placeholder} }}}}", content)

    def test_version_comes_from_the_tag(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, _ = self._release(root)
            out = root / "out"

            render_channel_manifests(manifest, out, tag="v2.1.0-rc.3")

            for spec in CHANNELS.values():
                content = (out / spec["output"]).read_text(encoding="utf-8")
                self.assertIn("2.1.0-rc.3", content)

    def test_scoop_and_winget_manifests_are_valid_json(self) -> None:
        """These two are parsed by their package managers, so they must parse."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, _ = self._release(root)
            out = root / "out"

            render_channel_manifests(manifest, out, tag="v2.0.0")

            scoop = json.loads((out / "pandora.json").read_text(encoding="utf-8"))
            self.assertEqual(scoop["version"], "2.0.0")
            self.assertEqual(len(scoop["hash"]), 64)

            winget = json.loads(
                (out / "AnisayakmitraIn.AGENT-PANDORA.yaml").read_text(
                    encoding="utf-8"
                )
            )
            self.assertEqual(winget["PackageVersion"], "2.0.0")
            self.assertEqual(
                len(winget["Installers"][0]["InstallerSha256"]), 64
            )

    def test_missing_artifact_is_refused_rather_than_rendered_without_a_hash(
        self,
    ) -> None:
        """A channel with no digest must fail, not ship an unusable manifest.

        The manifest here is valid but lists only the Linux artifact, so the
        Homebrew channel cannot find its macOS binary.
        """
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, digests = self._release(root)
            linux = "pandora-x86_64-unknown-linux-gnu"
            (root / "dist" / linux).write_bytes(b"linux binary\n")
            manifest.write_text(
                f"{hashlib.sha256(b'linux binary' + chr(10).encode()).hexdigest()}  {linux}\n",
                encoding="utf-8",
            )

            with self.assertRaisesRegex(ChannelManifestError, "no entry"):
                render_channel_manifests(manifest, root / "out", tag="v2.0.0")

            # Nothing may be published for a channel we could not fill in.
            self.assertFalse((root / "out" / "pandora.rb").exists())

    def test_empty_manifest_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, _ = self._release(root)
            manifest.write_text("", encoding="utf-8")

            with self.assertRaisesRegex(ChannelManifestError, "checksum manifest"):
                render_channel_manifests(manifest, root / "out", tag="v2.0.0")

    def test_download_urls_use_the_full_release_tag(self) -> None:
        """A URL built from the bare version would 404.

        Release tags carry a leading "v" and the version field does not, so the
        two must not be conflated. This was a real defect: rendering v2.0.0
        produced .../releases/download/2.0.0/, which does not exist.
        """
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, _ = self._release(root)
            out = root / "out"

            render_channel_manifests(manifest, out, tag="v2.0.0")

            for channel, spec in CHANNELS.items():
                with self.subTest(channel=channel):
                    content = (out / spec["output"]).read_text(encoding="utf-8")
                    self.assertIn("/releases/download/v2.0.0/", content)
                    self.assertNotIn("/releases/download/2.0.0/", content)

    def test_prerelease_tag_renders_into_urls(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, _ = self._release(root)
            out = root / "out"

            render_channel_manifests(manifest, out, tag="v2.0.0-rc.3")

            for spec in CHANNELS.values():
                content = (out / spec["output"]).read_text(encoding="utf-8")
                self.assertIn("/releases/download/v2.0.0-rc.3/", content)

    def test_invalid_tag_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, _ = self._release(root)

            for tag in ("2.0.0", "latest", "v2.0", "release-1"):
                with self.subTest(tag=tag):
                    with self.assertRaisesRegex(
                        ChannelManifestError, "invalid release tag"
                    ):
                        render_channel_manifests(
                            manifest, root / "out", tag=tag
                        )

    def test_unknown_placeholder_is_an_error(self) -> None:
        """A placeholder with no value must not pass through into a manifest."""
        with self.assertRaisesRegex(
            ChannelManifestError, "has no value"
        ):
            render_template('version "{{ NOPE }}"', {"VERSION": "1.0.0"})

    def test_manifests_state_that_binaries_are_not_os_signed(self) -> None:
        """Every channel manifest must not imply a signature it does not have."""
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, _ = self._release(root)
            out = root / "out"

            render_channel_manifests(manifest, out, tag="v2.0.0")

            for channel, spec in CHANNELS.items():
                with self.subTest(channel=channel):
                    content = (out / spec["output"]).read_text(encoding="utf-8")
                    lowered = content.lower()
                    self.assertIn("not", lowered)
                    self.assertNotIn("codesign --sign ", content)
                    self.assertNotIn("signtool", lowered)

    def test_rendering_is_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, _ = self._release(root)
            first, second = root / "a", root / "b"

            render_channel_manifests(manifest, first, tag="v2.0.0")
            render_channel_manifests(manifest, second, tag="v2.0.0")

            for spec in CHANNELS.values():
                self.assertEqual(
                    (first / spec["output"]).read_text(encoding="utf-8"),
                    (second / spec["output"]).read_text(encoding="utf-8"),
                )


class RenderTemplateTests(unittest.TestCase):
    def test_substitutes_every_placeholder(self) -> None:
        self.assertEqual(
            render_template(
                'v{{ VERSION }} {{ SHA256 }} {{ ARCH }}',
                {"VERSION": "1.2.3", "SHA256": "abc", "ARCH": "x64"},
            ),
            "v1.2.3 abc x64",
        )

    def test_leaves_unknown_syntax_alone(self) -> None:
        """Only UPPER_SNAKE placeholders are substituted."""
        self.assertEqual(
            render_template(
                "{ repo } {{ version }} {name}",
                {"VERSION": "1.0.0"},
            ),
            "{ repo } {{ version }} {name}",
        )


if __name__ == "__main__":
    unittest.main()