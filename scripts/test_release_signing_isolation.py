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
    def test_unprivileged_jobs_do_not_reference_platform_signing_secrets(self) -> None:
        workflow = workflow_text()

        for job_name in ("verify", "build", "build-desktop"):
            block = job(workflow, job_name)
            for secret in SIGNING_SECRETS:
                self.assertNotIn(secret, block, f"{job_name} exposes {secret}")
            self.assertNotIn("${{ secrets.", block, f"{job_name} exposes a repository secret")

    def test_native_signing_consumes_immutable_unsigned_build_output(self) -> None:
        workflow = workflow_text()
        build = job(workflow, "build")
        signing = job(workflow, "sign-native")

        self.assertIn("name: native-unsigned-${{ matrix.target }}", build)
        self.assertNotIn("- name: Sign native CLI", build)
        self.assertIn("needs: [verify, build]", signing)
        self.assertIn("environment: release-publication", signing)
        self.assertIn("if: contains(github.ref_name, '-rc.') || !contains(github.ref_name, '-')", signing)
        self.assertIn("name: native-unsigned-${{ matrix.target }}", signing)
        self.assertIn("name: native-signed-${{ matrix.target }}", signing)
        approval = signing[signing.index("- name: Enforce release approval") : signing.index("- name: Download unsigned native artifact")]
        macos = signing[signing.index("- name: Sign native CLI (macOS)") : signing.index("- name: Sign native CLI (Windows)")]
        windows = signing[signing.index("- name: Sign native CLI (Windows)") : signing.index("- name: Stage signed native artifact (Unix)")]
        for secret in ("PANDORA_RELEASE_CANDIDATE_APPROVED", "PANDORA_STABLE_RELEASE_APPROVED"):
            self.assertIn(secret, approval)
        for secret in (
            "PANDORA_APPLE_CERTIFICATE_BASE64",
            "PANDORA_APPLE_CERTIFICATE_PASSWORD",
            "APPLE_SIGNING_IDENTITY",
        ):
            self.assertIn(secret, macos)
            self.assertNotIn(secret, windows)
        for secret in (
            "PANDORA_WINDOWS_CERTIFICATE_BASE64",
            "PANDORA_WINDOWS_CERTIFICATE_PASSWORD",
        ):
            self.assertIn(secret, windows)
            self.assertNotIn(secret, macos)
        for secret in ("APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID"):
            self.assertNotIn(secret, signing, f"native signer must not receive {secret}")
        self.assertIn("codesign --verify --strict --verbose=2", signing)
        self.assertIn("Import-PfxCertificate", windows)
        self.assertIn("/sha1 $certificate.Thumbprint", windows)
        self.assertNotIn("/p $env:PANDORA_WINDOWS_CERTIFICATE_PASSWORD", windows)
        self.assertIn("signtool verify /pa /all /v", signing)

    def test_publish_selects_unsigned_beta_or_protected_signed_native_artifacts(self) -> None:
        workflow = workflow_text()
        staging = job(workflow, "stage-native")
        publish = job(workflow, "publish")

        self.assertIn("needs: [build, sign-native]", staging)
        self.assertIn("if: always()", staging)
        self.assertIn("pattern: native-unsigned-*", staging)
        self.assertIn("pattern: native-signed-*", staging)
        self.assertIn("name: release-native-${{ github.sha }}", staging)
        self.assertIn("needs: [stage-native, build-desktop]", publish)
        self.assertIn("name: release-native-${{ github.sha }}", publish)
        self.assertNotIn("pattern: native-*", publish)

    def test_desktop_build_is_secretless_and_rc_stable_remain_fail_closed(self) -> None:
        workflow = workflow_text()
        desktop = job(workflow, "build-desktop")
        publish = job(workflow, "publish")

        self.assertIn("name: desktop-unsigned-${{ matrix.artifact }}", desktop)
        self.assertNotIn("Import Apple signing identity", desktop)
        self.assertNotIn("Sign desktop bundles (Windows)", desktop)
        self.assertIn("pattern: desktop-unsigned-*", publish)
        self.assertIn("RC/stable release blocked: isolated desktop platform signing is not configured", publish)


if __name__ == "__main__":
    unittest.main()
