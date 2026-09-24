import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
SIGNING_SECRETS = (
    "PANDORA_RELEASE_CANDIDATE_APPROVED",
    "PANDORA_STABLE_RELEASE_APPROVED",
    "PANDORA_WINDOWS_CERTIFICATE_BASE64",
    "PANDORA_WINDOWS_CERTIFICATE_PASSWORD",
    "PANDORA_APPLE_CERTIFICATE_BASE64",
    "PANDORA_APPLE_CERTIFICATE_PASSWORD",
    "APPLE_SIGNING_IDENTITY",
    "APPLE_ID",
    "APPLE_PASSWORD",
    "APPLE_TEAM_ID",
)


def workflow_text() -> str:
    return WORKFLOW.read_text(encoding="utf-8")


def job(workflow: str, name: str) -> str:
    start = re.search(rf"^  {re.escape(name)}:\s*$", workflow, re.MULTILINE)
    if start is None:
        raise AssertionError(f"workflow job is missing: {name}")
    next_job = re.search(r"^  [A-Za-z0-9_-]+:\s*$", workflow[start.end() :], re.MULTILINE)
    end = start.end() + next_job.start() if next_job else len(workflow)
    return workflow[start.start() : end]


class ReleaseSigningIsolationTests(unittest.TestCase):
    def test_unprivileged_jobs_have_no_secrets_context(self) -> None:
        workflow = workflow_text()

        for job_name in (
            "verify",
            "build",
            "build-desktop",
            "stage-native",
            "release-gate",
        ):
            block = job(workflow, job_name)
            for secret in SIGNING_SECRETS:
                self.assertNotIn(secret, block, f"{job_name} exposes {secret}")
            self.assertIsNone(
                re.search(r"\bsecrets\b", block, re.IGNORECASE),
                f"{job_name} exposes a Secrets context",
            )
            self.assertNotIn("secrets: inherit", block, f"{job_name} inherits secrets")

    def test_rc_and_stable_fail_in_a_secretless_gate_before_protected_jobs(self) -> None:
        workflow = workflow_text()
        gate = job(workflow, "release-gate")
        signing = job(workflow, "sign-native")
        desktop = job(workflow, "build-desktop")
        staging = job(workflow, "stage-native")
        publish = job(workflow, "publish")

        self.assertIn("needs: verify", gate)
        self.assertIn("permissions:\n      contents: read", gate)
        self.assertIn(
            "if: contains(github.ref_name, '-rc.') || !contains(github.ref_name, '-')",
            gate,
        )
        self.assertIn(
            "RC/stable release blocked: isolated desktop platform signing is not configured",
            gate,
        )
        self.assertIn("exit 1", gate)
        self.assertNotIn("exit 0", gate)
        self.assertNotIn("environment:", gate)
        self.assertIn("release-gate", signing)
        self.assertIn("release-gate", desktop)
        self.assertIn("release-gate", staging)
        self.assertIn("release-gate", publish)
        self.assertIn("needs: [verify, build, release-gate]", signing)
        self.assertIn("needs: [verify, build, release-gate]", desktop)
        self.assertIn("needs: [build, sign-native, release-gate]", staging)
        self.assertIn("needs: [release-gate, stage-native, build-desktop]", publish)
        self.assertNotIn("Block RC and stable", publish)

    def test_signer_never_executes_untrusted_build_output(self) -> None:
        signing = job(workflow_text(), "sign-native")

        self.assertNotIn("--version", signing)
        self.assertNotIn("setup", signing)
        self.assertNotIn("checkout", signing)
        macos = signing[
            signing.index("- name: Sign native CLI (macOS)") : signing.index(
                "- name: Sign native CLI (Windows)"
            )
        ]
        windows = signing[
            signing.index("- name: Sign native CLI (Windows)") : signing.index(
                "- name: Upload signed native artifact"
            )
        ]
        self.assertIn('cp "$RUNNER_TEMP/native-unsigned/${{ matrix.artifact }}"', macos)
        self.assertIn('"$RUNNER_TEMP/native-release/${{ matrix.artifact }}"', macos)
        self.assertIn('codesign --verify --strict --verbose=2 "$staged"', macos)
        self.assertIn('Copy-Item -LiteralPath $source', windows)
        self.assertIn('& signtool verify /pa /all /v $staged', windows)
        self.assertIn("Import-PfxCertificate", windows)
        self.assertIn("/sha1 $certificate.Thumbprint", windows)
        self.assertNotIn("/p $env:PANDORA_WINDOWS_CERTIFICATE_PASSWORD", windows)
        self.assertNotIn("-p $env:PANDORA_WINDOWS_CERTIFICATE_PASSWORD", windows)
        self.assertIn(
            "path: ${{ runner.temp }}/native-release/${{ matrix.artifact }}",
            signing,
        )

        mac_copy = macos.index(
            'cp "$RUNNER_TEMP/native-unsigned/${{ matrix.artifact }}"'
        )
        mac_sign = macos.index(
            'codesign --force --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$staged"'
        )
        mac_import = macos.index("security import")
        mac_verify = macos.index('codesign --verify --strict --verbose=2 "$staged"')
        self.assertLess(mac_copy, mac_import)
        self.assertLess(mac_import, mac_sign)
        self.assertLess(mac_sign, mac_verify)

        windows_copy = windows.index("Copy-Item -LiteralPath $source")
        windows_import = windows.index("Import-PfxCertificate")
        windows_sign = windows.index("& signtool sign")
        windows_verify = windows.index("& signtool verify /pa /all /v $staged")
        self.assertLess(windows_copy, windows_import)
        self.assertLess(windows_import, windows_sign)
        self.assertLess(windows_sign, windows_verify)

    def test_apple_signature_requires_developer_id_and_expected_team(self) -> None:
        signing = job(workflow_text(), "sign-native")
        macos = signing[
            signing.index("- name: Sign native CLI (macOS)") : signing.index(
                "- name: Sign native CLI (Windows)"
            )
        ]
        windows = signing[
            signing.index("- name: Sign native CLI (Windows)") : signing.index(
                "- name: Upload signed native artifact"
            )
        ]

        self.assertIn("APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}", macos)
        self.assertIn("codesign -dv --verbose=4", macos)
        self.assertIn("TeamIdentifier=$APPLE_TEAM_ID", macos)
        self.assertIn("Authority=Developer ID Application:", macos)
        self.assertNotIn("APPLE_TEAM_ID", windows)
        self.assertNotIn("APPLE_ID", signing)
        self.assertNotIn("APPLE_PASSWORD", signing)
        self.assertNotIn("APPLE_ID", macos)
        self.assertNotIn("APPLE_PASSWORD", macos)

    def test_linux_is_not_represented_as_signed(self) -> None:
        workflow = workflow_text()
        signing = job(workflow, "sign-native")
        staging = job(workflow, "stage-native")

        self.assertNotIn("ubuntu-latest", signing)
        self.assertNotIn("x86_64-unknown-linux-gnu", signing)
        self.assertIn(
            "name: native-unsigned-x86_64-unknown-linux-gnu",
            staging,
        )
        self.assertIn("pattern: native-signed-*", staging)

    def test_publish_selects_unsigned_beta_or_protected_signed_native_artifacts(self) -> None:
        workflow = workflow_text()
        staging = job(workflow, "stage-native")
        publish = job(workflow, "publish")

        self.assertIn("needs: [build, sign-native, release-gate]", staging)
        self.assertIn("needs.release-gate.result == 'success'", staging)
        self.assertIn("pattern: native-unsigned-*", staging)
        self.assertIn("name: release-native-${{ github.sha }}", staging)
        self.assertIn("needs: [release-gate, stage-native, build-desktop]", publish)
        self.assertIn("name: release-native-${{ github.sha }}", publish)
        self.assertNotIn("pattern: native-*", publish)

    def test_desktop_build_is_secretless_and_documentation_stays_fail_closed(self) -> None:
        workflow = workflow_text()
        desktop = job(workflow, "build-desktop")
        publish = job(workflow, "publish")
        production = (ROOT / "docs" / "PRODUCTION.md").read_text(encoding="utf-8")
        releases = (ROOT / "RELEASES.md").read_text(encoding="utf-8")

        self.assertIn("name: desktop-unsigned-${{ matrix.artifact }}", desktop)
        self.assertNotIn("Import Apple signing identity", desktop)
        self.assertNotIn("Sign desktop bundles (Windows)", desktop)
        self.assertIn("pattern: desktop-unsigned-*", publish)
        self.assertIn("fail before public release", production)
        self.assertIn("not reachable in the current workflow", releases)
        self.assertNotIn(
            "The four published-package smoke jobs independently re-download checksum-bound",
            releases,
        )


if __name__ == "__main__":
    unittest.main()
