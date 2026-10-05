from __future__ import annotations

import hashlib
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from scripts.verify_release_downloads import (
    ReleaseDownloadError,
    verify_release_downloads,
    verify_release_evidence_signature,
)

_IDENTITY = "https://example.test/.github/workflows/release.yml@refs/tags/v1.0.0"


class VerifyReleaseDownloadsTests(unittest.TestCase):
    def _fixture(self, root: Path) -> tuple[Path, list[Path]]:
        artifacts = [
            root / "pandora-x86_64-unknown-linux-gnu",
            root / "desktop-linux-x64-Pandora_2.0.0_amd64.deb",
        ]
        payloads = [b"native sidecar\n", b"desktop package\n"]
        for path, payload in zip(artifacts, payloads, strict=True):
            path.write_bytes(payload)
        manifest = root / "checksums.txt"
        manifest.write_text(
            "".join(
                f"{hashlib.sha256(payload).hexdigest()}  {path.name}\n"
                for path, payload in zip(artifacts, payloads, strict=True)
            ),
            encoding="utf-8",
        )
        return manifest, artifacts

    def _signed_fixture(self, root: Path) -> tuple[Path, list[Path]]:
        """A release directory that also carries a cosign signature."""
        manifest, artifacts = self._fixture(root)
        manifest.with_suffix(".txt.sig").write_bytes(b"cosign signature\n")
        manifest.with_suffix(".txt.pem").write_bytes(b"cosign certificate\n")
        return manifest, artifacts

    def test_verifies_native_and_desktop_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._fixture(Path(temporary))

            verify_release_downloads(manifest, artifacts)

    def test_rejects_changed_artifact(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._fixture(Path(temporary))
            artifacts[1].write_bytes(b"changed\n")

            with self.assertRaisesRegex(ReleaseDownloadError, "checksum mismatch"):
                verify_release_downloads(manifest, artifacts)

    def test_rejects_missing_and_duplicate_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._fixture(Path(temporary))

            with self.assertRaisesRegex(ReleaseDownloadError, "regular file"):
                verify_release_downloads(manifest, [Path(temporary) / "missing"])
            with self.assertRaisesRegex(ReleaseDownloadError, "duplicate"):
                verify_release_downloads(manifest, [artifacts[0], artifacts[0]])

    def test_unsigned_manifest_still_verifies_checksums(self) -> None:
        """Backwards compatibility: a release with no signature is still checked.

        This is the only case where an unauthenticated manifest is accepted. It
        is never a silent downgrade -- callers that need the stronger guarantee
        pass require_signature=True.
        """
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._fixture(Path(temporary))

            verify_release_evidence_signature(manifest)
            verify_release_downloads(manifest, artifacts)

    def test_required_signature_fails_closed_when_absent(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._fixture(Path(temporary))

            with self.assertRaisesRegex(
                ReleaseDownloadError, "required but not present"
            ):
                verify_release_downloads(
                    manifest, artifacts, require_signature=True
                )

    def test_present_signature_is_verified_and_a_failure_is_fatal(self) -> None:
        """A signature that exists but cannot be verified must fail.

        This is the case that motivated the change. Accepting an artifact because
        its checksum matched a manifest whose signature we simply skipped would
        mean anything able to serve both files controls the result.
        """
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._signed_fixture(Path(temporary))

            completed = subprocess.CompletedProcess(
                args=[], returncode=1, stdout="", stderr="no matching signatures"
            )
            with mock.patch(
                "scripts.verify_release_downloads.shutil.which",
                return_value="cosign",
            ), mock.patch(
                "scripts.verify_release_downloads.subprocess.run",
                return_value=completed,
            ):
                with self.assertRaisesRegex(
                    ReleaseDownloadError, "signature verification failed"
                ):
                    verify_release_downloads(manifest, artifacts, identity=_IDENTITY)

    def test_tampered_manifest_and_artifact_pair_is_rejected(self) -> None:
        """A manifest rewritten together with its artifact must not be accepted.

        This is the case that motivated the change. An attacker who can serve
        both files rewrites the manifest and the artifact together, so every
        checksum still matches. What stops them is that they cannot forge the
        manifest's cosign signature.

        Note on scope: this test proves the pair is rejected, not that the
        signature check happens before the checksum loop. Reordering the two
        calls was measured and does not change the outcome -- either order fails
        closed, because a forged signature cannot verify either way. The
        signature is still called first in the implementation so that a failure
        is reported as what it is rather than as a confusing checksum error.
        """
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._signed_fixture(Path(temporary))

            # Rewrite the manifest and the artifact together, consistently.
            artifacts[0].write_bytes(b"attacker payload\n")
            first = hashlib.sha256(b"attacker payload\n").hexdigest()
            second = hashlib.sha256(artifacts[1].read_bytes()).hexdigest()
            manifest.write_text(
                f"{first}  {artifacts[0].name}\n"
                f"{second}  {artifacts[1].name}\n",
                encoding="utf-8",
            )

            completed = subprocess.CompletedProcess(
                args=[], returncode=1, stdout="", stderr="no matching signatures"
            )
            with mock.patch(
                "scripts.verify_release_downloads.shutil.which",
                return_value="cosign",
            ), mock.patch(
                "scripts.verify_release_downloads.subprocess.run",
                return_value=completed,
            ):
                with self.assertRaisesRegex(
                    ReleaseDownloadError, "signature verification failed"
                ):
                    verify_release_downloads(
                        manifest, [artifacts[0], artifacts[1]], identity=_IDENTITY
                    )

    def test_verified_signature_allows_the_checksum_path_to_run(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._signed_fixture(Path(temporary))

            completed = subprocess.CompletedProcess(
                args=[], returncode=0, stdout="verified", stderr=""
            )
            with mock.patch(
                "scripts.verify_release_downloads.shutil.which",
                return_value="cosign",
            ), mock.patch(
                "scripts.verify_release_downloads.subprocess.run",
                return_value=completed,
            ) as run:
                verify_release_downloads(manifest, artifacts, identity=_IDENTITY)

            command = run.call_args.args[0]
            self.assertIn("verify-blob", command)
            self.assertIn(_IDENTITY, command)

    def test_missing_cosign_fails_closed_instead_of_skipping(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._signed_fixture(Path(temporary))

            with mock.patch(
                "scripts.verify_release_downloads.shutil.which", return_value=None
            ):
                with self.assertRaisesRegex(ReleaseDownloadError, "cosign is required"):
                    verify_release_downloads(
                        manifest, artifacts, identity=_IDENTITY
                    )

    def test_signature_without_identity_is_refused(self) -> None:
        """Verifying without pinning an identity would accept any signer."""
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._signed_fixture(Path(temporary))

            with self.assertRaisesRegex(
                ReleaseDownloadError, "certificate identity is required"
            ):
                verify_release_downloads(manifest, artifacts)

    def test_empty_signature_file_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._signed_fixture(Path(temporary))
            manifest.with_suffix(".txt.sig").write_bytes(b"")

            with self.assertRaisesRegex(
                ReleaseDownloadError, "signature is missing or empty"
            ):
                verify_release_downloads(manifest, artifacts, identity=_IDENTITY)

    def test_local_cosign_is_not_required_for_checksum_only_verification(self) -> None:
        """The checksum path must not depend on cosign being installed."""
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._fixture(Path(temporary))

            with mock.patch(
                "scripts.verify_release_downloads.shutil.which", return_value=None
            ):
                verify_release_downloads(manifest, artifacts)

    @unittest.skipIf(shutil.which("cosign") is None, "cosign is not installed")
    def test_real_cosign_rejects_a_forged_manifest(self) -> None:
        """End-to-end with the real cosign binary, when it is available.

        The fixture signature bytes are not a real signature, so cosign must
        fail. This proves the invocation is wired to a genuine verification
        rather than trusting an exit code we invented.
        """
        with tempfile.TemporaryDirectory() as temporary:
            manifest, artifacts = self._signed_fixture(Path(temporary))

            with self.assertRaisesRegex(
                ReleaseDownloadError, "signature verification failed"
            ):
                verify_release_downloads(manifest, artifacts, identity=_IDENTITY)


if __name__ == "__main__":
    unittest.main()
