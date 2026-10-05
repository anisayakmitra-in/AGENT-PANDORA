import json
import re
import unittest
from pathlib import Path

from scripts.release_notes import extract_release_notes


ROOT = Path(__file__).resolve().parent.parent


class ReleaseNotesTests(unittest.TestCase):
    def test_extracts_only_the_requested_tag_section(self) -> None:
        changelog = """# Changelog

## Unreleased

- Not published.

## v2.0.0-alpha.2

Published notes.

### Shipped

- One feature.

## v2.0.0-alpha.1

Older notes.
"""

        self.assertEqual(
            extract_release_notes(changelog, "v2.0.0-alpha.2"),
            "Published notes.\n\n### Shipped\n\n- One feature.",
        )

    def test_rejects_a_missing_tag(self) -> None:
        with self.assertRaisesRegex(ValueError, "missing"):
            extract_release_notes("# Changelog\n", "v2.0.0-alpha.2")

    def test_rejects_an_empty_tag_section(self) -> None:
        with self.assertRaisesRegex(ValueError, "empty"):
            extract_release_notes("# Changelog\n\n## v2.0.0-alpha.2\n", "v2.0.0-alpha.2")

    def test_release_workflow_verifies_and_publishes_release_evidence(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        verify = workflow.index("- name: Verify checksum signature")
        evidence = workflow.index("- name: Generate release evidence index")
        publish = workflow.index("- name: Publish GitHub release")
        self.assertLess(verify, evidence)
        self.assertLess(evidence, publish)
        self.assertIn("cosign verify-blob", workflow)
        self.assertIn('release_evidence.py "$GITHUB_REF_NAME"', workflow)
        self.assertIn("release-evidence.json", workflow)

    def test_release_workflow_publishes_tag_scoped_notes(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn(
            'python scripts/release_notes.py "$GITHUB_REF_NAME" > "$RUNNER_TEMP/release-notes.md"',
            workflow,
        )
        self.assertIn("body_path: ${{ runner.temp }}/release-notes.md", workflow)
        self.assertNotIn("body_path: CHANGELOG.md", workflow)

    def test_publish_uses_the_exact_npm_tarball_built_by_the_verified_job(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        verify_start = workflow.index("  verify:")
        build_start = workflow.index("\n  build:", verify_start)
        verify = workflow[verify_start:build_start]
        publish_start = workflow.index("  publish:")
        publish_end = workflow.find("\n  smoke-install:", publish_start)
        if publish_end == -1:
            publish_end = len(workflow)
        publish = workflow[publish_start:publish_end]

        build = verify.index("- name: Build TypeScript launcher boundary")
        pack = verify.index("- name: Package verified npm launcher")
        upload = verify.index("- name: Upload verified npm launcher")
        self.assertLess(build, pack)
        self.assertLess(pack, upload)
        self.assertIn("npm pack --ignore-scripts", verify)
        self.assertIn("name: npm-launcher-${{ github.sha }}", verify)
        self.assertIn("npm ci --ignore-scripts && npm run build", verify)
        self.assertIn("git diff --exit-code -- npm/pandora-cli/lib", verify)

        download = publish.index("- name: Download verified npm launcher")
        copy = publish.index('npm_packages=("$RUNNER_TEMP"/pandora-npm/pandora-agent-*.tgz)')
        checksums = publish.index("- name: Generate checksums")
        self.assertLess(download, copy)
        self.assertLess(copy, checksums)
        self.assertIn('test "${#npm_packages[@]}" -eq 1', publish)
        self.assertNotIn("npm pack", publish)

    def test_npm_launcher_does_not_execute_install_hooks(self) -> None:
        package = json.loads(
            (ROOT / "npm" / "pandora-cli" / "package.json").read_text(
                encoding="utf-8"
            )
        )
        scripts = package.get("scripts", {})
        for hook in ("postinstall", "install", "prepare", "prepack", "prepublishOnly"):
            self.assertNotIn(hook, scripts)
        self.assertEqual(package["bin"]["pandora"], "bin/pandora.js")

    def test_desktop_is_cancelled_and_absent_from_the_cli_pipeline(self) -> None:
        """Permanent cancellation, asserted rather than left to a variable.

        The desktop job used to sit in CI behind `vars.PANDORA_DESKTOP_CI`, which
        nothing ever set. That read as though reactivation were a variable flip,
        which is not true: reviving desktop packaging needs its own build, signing,
        and evidence work. With the cancellation made permanent the job is
        removed, so the CLI pipeline cannot grow a desktop context by accident and
        no one can set a variable to re-enable it.
        """
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        self.assertNotIn(
            "  desktop:",
            workflow,
            "the cancelled desktop job must not remain in the CLI workflow",
        )
        self.assertNotIn(
            "PANDORA_DESKTOP_CI",
            workflow,
            "the desktop reactivation variable must not remain in the CLI workflow",
        )
        self.assertNotIn(
            "apps/pandora-desktop",
            workflow,
            "the CLI workflow must not reference the cancelled desktop tree",
        )
        self.assertNotIn("needs: desktop", workflow)
        # The source itself is retained for downstream reuse, so the tree stays.
        self.assertTrue(
            (ROOT / "apps" / "pandora-desktop").is_dir(),
            "the cancelled desktop source is retained, not deleted",
        )

    def test_agent_pipeline_binds_promotion_to_tracked_artifact_and_evidence_bytes(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "agent-pipeline.yml").read_text(
            encoding="utf-8"
        )

        validate_start = workflow.index("  validate:")
        promotion_start = workflow.index("  promotion-gate:")
        validate = workflow[validate_start:promotion_start]
        promotion = workflow[promotion_start:]

        manifest = validate.index("- name: Create canonical pipeline evidence manifest")
        input_verify = validate.index("- name: Verify submitted digests before requesting promotion approval")
        upload = validate.index("- name: Upload pipeline evidence")
        self.assertLess(manifest, input_verify)
        self.assertLess(input_verify, upload)
        self.assertIn("artifact_path:", workflow[:promotion_start])
        self.assertIn("promotion_id:", workflow[:promotion_start])
        self.assertNotIn("approval_id:", workflow[:promotion_start])
        for trigger_path in (
            '"scripts/agent_pipeline_evidence.py"',
            '"scripts/test_agent_pipeline_evidence.py"',
        ):
            self.assertIn(trigger_path, workflow[:validate_start])
        self.assertIn("scripts/agent_pipeline_evidence.py create", validate)
        self.assertIn('--repository-root "$GITHUB_WORKSPACE"', validate)
        self.assertIn("scripts/agent_pipeline_evidence.py verify", validate)
        self.assertIn("canary_stops_before_activation_tested", validate)

        download = promotion.index("- name: Download exact pipeline evidence")
        identity = promotion.index("python scripts/release_identity.py \"$RELEASE_TAG\"")
        verify = promotion.index("- name: Reverify exact artifact and evidence digests after reviewer approval")
        tag = promotion.index("- name: Create the one approved tag")
        self.assertLess(download, verify)
        self.assertLess(identity, tag)
        self.assertLess(verify, tag)
        self.assertIn('python scripts/release_identity.py "$RELEASE_TAG"', promotion)
        self.assertIn('"promotion_id": os.environ["PROMOTION_ID"]', promotion)
        self.assertIn("artifact_path", promotion[verify:tag])
        self.assertIn("evidence-manifest.json", promotion[verify:])
        self.assertIn('"artifact_path": os.environ["ARTIFACT_PATH"]', promotion)

    def test_release_workflow_smokes_native_cli_before_uploading_assets(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        smoke_step = workflow.index("- name: Smoke test native CLI")
        unix_stage = workflow.index("- name: Stage Unix artifact")
        windows_stage = workflow.index("- name: Stage Windows artifact")

        self.assertLess(smoke_step, unix_stage)
        self.assertLess(smoke_step, windows_stage)
        self.assertIn('expected="pandora ${GITHUB_REF_NAME#v}"', workflow)
        self.assertIn(
            '$expected = "pandora " + $env:GITHUB_REF_NAME.Substring(1)', workflow
        )

    def test_release_workflow_does_not_os_sign_any_release_artifact(self) -> None:
        """No release channel may require an OS signature.

        Pandora is a terminal CLI, so Authenticode, Developer ID signing and
        notarization were removed from the release process. These assertions are
        the regression guard: they fail if a codesign/signtool/stapler/spctl gate
        or a vendor-signing receipt requirement is reintroduced anywhere in the
        workflow. Integrity comes from checksums.txt, its cosign signature, and
        build attestations instead, and those are asserted elsewhere.
        """
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        for marker in (
            "signtool sign",
            "signtool verify /pa",
            "codesign --force",
            "codesign --verify",
            "codesign -dv",
            "xcrun stapler validate",
            "spctl --assess",
            "Import-PfxCertificate",
            "Get-AuthenticodeSignature",
            "APPLE_SIGNING_IDENTITY",
            "PANDORA_APPLE_CERTIFICATE_BASE64",
            "PANDORA_WINDOWS_CERTIFICATE_BASE64",
            "APPLE_TEAM_ID",
            "Authority=Developer ID Application:",
            "signing_required",
            "signingRequired",
            "platform_signing_required",
            "signing-receipt",
        ):
            with self.subTest(marker=marker):
                self.assertNotIn(marker, workflow)

        # The sign-native job must be gone, replaced by approval only.
        self.assertNotIn("  sign-native:", workflow)
        self.assertNotIn("native-signed-", workflow)
        self.assertIn("  approve-release:", workflow)

        # Provenance that replaces signing must remain present.
        self.assertIn("actions/attest-build-provenance@", workflow)
        self.assertIn("cosign sign-blob", workflow)
        self.assertIn("cosign verify-blob", workflow)
        self.assertIn("checksums.txt.sig", workflow)
        self.assertIn("checksums.txt.pem", workflow)

    def test_every_verifying_job_can_actually_reach_cosign(self) -> None:
        """A verification that cannot run is not a verification.

        Each job that calls verify_release_downloads.py with
        --require-signature shells out to cosign. If cosign is not installed in
        that job the check fails closed on every release, and if a future edit
        makes it skip instead, it verifies nothing. So each verifying job must
        install cosign.
        """
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        # Map every job to whether it requires a signature and whether it can run
        # the verification. Iterating the raw text keeps this honest when jobs
        # are added or renamed.
        jobs: dict[str, list[str]] = {}
        current: list[str] | None = None
        for line in workflow.splitlines():
            job_match = re.match(r"^  ([a-z][a-z0-9-]*):\s*$", line)
            if job_match:
                current = jobs.setdefault(job_match.group(1), [])
                continue
            if current is not None:
                current.append(line)

        verifying = [
            name
            for name, body in jobs.items()
            if any("--require-signature" in line for line in body)
        ]
        self.assertTrue(verifying, "no job requires a manifest signature")

        for name in verifying:
            body = "\n".join(jobs[name])
            with self.subTest(job=name):
                self.assertIn(
                    "sigstore/cosign-installer@",
                    body,
                    f"{name} requires a manifest signature but never installs cosign, "
                    "so it can only ever fail or skip",
                )
                self.assertIn("verify_release_downloads.py", body)

        # The publish job is the one that signs, so it installs cosign too even
        # though it does not pass --require-signature to the script.
        self.assertIn("sigstore/cosign-installer@", "\n".join(jobs["publish"]))

    def test_published_verification_enforces_the_manifest_signature(self) -> None:
        """The release must prove it signed what it published.

        Without OS signing, the cosign signature over checksums.txt and the
        build attestation are the entire integrity story. If a smoke job verifies
        only checksums, a signing regression passes the release while every
        downstream install fails closed. These assertions pin both.
        """
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        # Every verify_release_downloads.py call site must demand the signature.
        call_sites = workflow.count("verify_release_downloads.py")
        required_sites = workflow.count("--require-signature")
        identities = workflow.count("--certificate-identity")
        self.assertGreaterEqual(call_sites, 4)
        # smoke-install exercises the installer path rather than this script, so
        # the script call sites are the desktop smoke and rollback ones.
        self.assertGreaterEqual(required_sites, 4)
        self.assertGreaterEqual(identities, 4)

        # Verification pins an exact identity. A regexp would let any tag, or
        # any other workflow in the repo, satisfy the check.
        publish = workflow[workflow.index("\n  publish:") : workflow.index(
            "\n  smoke-install:"
        )]
        self.assertNotIn("--certificate-identity-regexp", publish)
        self.assertIn("cosign verify-blob", publish)
        self.assertIn(
            "https://github.com/anisayakmitra-in/AGENT-PANDORA/.github/workflows"
            "/release.yml@refs/tags/${{ github.ref_name }}",
            workflow,
        )

        # The attestation is actually verified, not merely produced.
        self.assertIn("gh attestation verify", workflow)
        self.assertIn("--repo anisayakmitra-in/AGENT-PANDORA", workflow)

        # Smoke jobs need cosign available, or the verification would always fail.
        smoke_install = workflow[
            workflow.index("  smoke-install:") : workflow.index(
                "\n  smoke-desktop:"
            )
        ]
        smoke_desktop = workflow[workflow.index("  smoke-desktop:") :]
        for job in (smoke_install, smoke_desktop):
            with self.subTest(job=job.strip().splitlines()[0]):
                self.assertIn("sigstore/cosign-installer@", job)

    def test_release_candidate_and_stable_still_require_human_approval(self) -> None:
        """Removing signing must not remove the human gate.

        The signing job also carried the RC/stable approval check and the
        `release-publication` environment. Both survive in `approve-release`.
        """
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        gate_start = workflow.index("  release-gate:")
        gate_end = workflow.index("\n  build:", gate_start)
        gate = workflow[gate_start:gate_end]
        approve_start = workflow.index("  approve-release:")
        stage_start = workflow.index("\n  stage-native:", approve_start)
        publish_start = workflow.index("\n  publish:", approve_start)
        approve = workflow[approve_start:stage_start]
        publish = workflow[publish_start:]

        self.assertIn("needs: verify", gate)
        self.assertIn('test "${{ needs.verify.outputs.scope }}" = "cli-only"', gate)
        self.assertIn(
            'test "${{ needs.verify.outputs.desktop_required }}" = "false"', gate
        )
        self.assertNotIn("environment:", gate)
        self.assertIn("Enforce release approval", approve)
        self.assertIn("environment: release-publication", approve)
        self.assertIn("PANDORA_RELEASE_CANDIDATE_APPROVED:", approve)
        self.assertIn("PANDORA_STABLE_RELEASE_APPROVED:", approve)
        self.assertIn('if [[ "$version" == *-rc.* ]]', approve)
        self.assertIn("needs: [verify, build, release-gate]", approve)
        self.assertIn(
            "needs: [verify, release-gate, stage-native, build-desktop]", publish
        )
        self.assertNotIn("Block RC and stable", publish)
        self.assertIn("environment: release-publication", workflow)
        self.assertIn("Validate required native accessibility evidence", workflow)
        self.assertIn("scripts/accessibility_evidence.py", workflow)

        # Every channel stages the same unsigned-by-OS artifacts now.
        stage = workflow[stage_start:publish_start]
        self.assertIn("pattern: native-unsigned-*", stage)
        self.assertNotIn("pattern: native-signed-*", stage)

    def test_release_workflow_smokes_published_installers_on_fresh_runners(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("smoke-install:", workflow)
        self.assertIn("needs: publish", workflow)
        for operating_system in (
            "ubuntu-latest",
            "macos-15-intel",
            "macos-14",
            "windows-latest",
        ):
            self.assertIn(f"- os: {operating_system}", workflow)
        self.assertIn("releases/download/${GITHUB_REF_NAME}/install.sh", workflow)
        self.assertIn("releases/download/$env:GITHUB_REF_NAME/install.ps1", workflow)
        self.assertNotIn('PANDORA_INSTALL_DIR: ${{ runner.temp }}', workflow)
        self.assertIn(
            'export PANDORA_INSTALL_DIR="$RUNNER_TEMP/pandora-bin"', workflow
        )
        self.assertIn(
            '$env:PANDORA_INSTALL_DIR = Join-Path $env:RUNNER_TEMP "pandora-bin"',
            workflow,
        )
        self.assertIn('PANDORA_VERSION: ${{ github.ref_name }}', workflow)
        self.assertIn('expected="pandora ${GITHUB_REF_NAME#v}"', workflow)
        self.assertIn(
            '$expected = "pandora " + $env:GITHUB_REF_NAME.Substring(1)', workflow
        )

    def test_release_workflow_exercises_full_lifecycle(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn(
            'predecessor="$(python scripts/release_predecessor.py "$GITHUB_REF_NAME")"',
            workflow,
        )
        self.assertIn(
            "$predecessor = (& python scripts/release_predecessor.py "
            "$env:GITHUB_REF_NAME).Trim()",
            workflow,
        )
        for command in (
            "PANDORA_VERSION=\"$predecessor\" sh \"$installer\"",
            "PANDORA_VERSION=\"$GITHUB_REF_NAME\" sh \"$installer\"",
            '"$cli" update --artifact "$cli"',
            '"$cli" update --rollback',
            '"$cli" setup --json',
            '"$cli" doctor --json',
            '"$cli" backup create --output "$backup" --json',
            '"$cli" backup inspect --input "$backup" --json',
            '"$cli" backup restore --input "$backup" --yes --json',
            '"$cli" uninstall --dry-run --json',
            '"$cli" uninstall --yes --json',
            "$env:PANDORA_VERSION = $predecessor",
            "$env:PANDORA_VERSION = $env:GITHUB_REF_NAME",
            "update --artifact $cli",
            "update --rollback",
            "setup --json",
            "doctor --json",
            "backup create --output $backup --json",
            "backup inspect --input $backup --json",
            "backup restore --input $backup --yes --json",
            "uninstall --dry-run --json",
            "uninstall --yes --json",
        ):
            self.assertIn(command, workflow)
        self.assertIn("checksums.txt", workflow)
        self.assertIn("sentinel.txt", workflow)

    def test_release_workflow_smokes_published_desktop_packages(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("smoke-desktop:", workflow)
        self.assertIn(
            "Exercise published desktop package on ${{ matrix.platform }}", workflow
        )
        for bundle in (
            "desktop-linux-x64-*.deb",
            "desktop-macos-x64-*.dmg",
            "desktop-macos-arm64-*.dmg",
            "desktop-windows-x64-*.msi",
        ):
            self.assertIn(bundle, workflow)
        self.assertIn("gh release download", workflow)
        self.assertIn("verify_release_downloads.py", workflow)
        self.assertIn("PANDORA_DESKTOP_SOURCE_SIDECAR", workflow)
        self.assertIn("PANDORA_DESKTOP_BUNDLE_ROOT", workflow)
        self.assertIn("PANDORA_DESKTOP_SYSTEM_INSTALL_LIFECYCLE", workflow)
        self.assertIn("npm run verify:bundle-lifecycle", workflow)
        # Published desktop artifacts are checksum-verified but never OS-signed,
        # so the smoke job records that posture instead of asserting a signature.
        self.assertIn("platform-signature-verification.json", workflow)
        self.assertNotIn("Get-AuthenticodeSignature", workflow)
        self.assertNotIn('codesign --verify --strict --verbose=2 "$native"', workflow)
        self.assertNotIn("xcrun stapler validate", workflow)
        self.assertNotIn("spctl --assess --type execute", workflow)

    def test_stable_release_records_honest_post_publication_rollback_state(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("stable-rollback-evidence:", workflow)
        self.assertIn("stable-desktop-rollback:", workflow)
        self.assertIn(
            "needs: [verify, smoke-install, smoke-desktop, stable-desktop-rollback]",
            workflow,
        )
        self.assertIn("scripts/stable_rollback_evidence.py", workflow)
        self.assertIn("--stable-only", workflow)
        self.assertIn("PANDORA_DESKTOP_PREDECESSOR_SIDECAR:", workflow)
        self.assertIn("PANDORA_DESKTOP_CURRENT_SIDECAR:", workflow)
        self.assertIn("npm run verify:bundle-upgrade-lifecycle", workflow)
        self.assertIn("stable-rollback-${{ github.ref_name }}", workflow)
