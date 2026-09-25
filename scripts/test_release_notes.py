import json
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

    def test_desktop_ci_job_is_retained_but_not_an_active_gate(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )
        start = workflow.index("  desktop:")
        desktop = workflow[start:]
        self.assertIn("vars.PANDORA_DESKTOP_CI == 'enabled'", desktop)
        self.assertIn("apps/pandora-desktop", desktop)
        self.assertNotIn("needs: desktop", workflow)

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

    def test_release_workflow_builds_desktop_secretless_and_blocks_unsafe_rc_stable(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        gate_start = workflow.index("  release-gate:")
        gate_end = workflow.index("\n  build:", gate_start)
        gate = workflow[gate_start:gate_end]
        desktop_start = workflow.index("  build-desktop:")
        publish_start = workflow.index("\n  publish:", desktop_start)
        desktop = workflow[desktop_start:publish_start]
        publish = workflow[publish_start:]

        build = desktop.index("- name: Build desktop bundle")
        lifecycle = desktop.index("- name: Verify desktop bundle lifecycle")
        stage = desktop.index("- name: Stage desktop artifacts (Unix)")
        self.assertLess(build, lifecycle)
        self.assertLess(lifecycle, stage)
        self.assertNotIn("Import Apple signing identity", desktop)
        self.assertNotIn("Sign desktop bundles (Windows)", desktop)
        self.assertNotIn("${{ secrets.", desktop)
        self.assertIn("name: desktop-unsigned-${{ matrix.artifact }}", desktop)
        self.assertIn("npm run verify:bundle-lifecycle", desktop)
        self.assertIn("patchelf xvfb", desktop)
        self.assertIn("target/release/bundle", desktop)
        self.assertNotIn("apps/pandora-desktop/src-tauri/target/release/bundle", desktop)
        self.assertIn("name: native-unsigned-${{ matrix.target }}", desktop)
        self.assertIn("PANDORA_SIDECAR_SOURCE:", desktop)
        self.assertIn("PANDORA_DESKTOP_SOURCE_SIDECAR:", desktop)
        self.assertIn("Verify desktop system install lifecycle", desktop)
        self.assertIn("PANDORA_DESKTOP_SYSTEM_INSTALL_LIFECYCLE: \"1\"", desktop)

        self.assertIn("needs: verify", gate)
        self.assertIn("permissions:\n      contents: read", gate)
        self.assertIn(
            "if: contains(github.ref_name, '-rc.') || !contains(github.ref_name, '-')",
            gate,
        )
        block = gate.index("Block RC and stable until isolated desktop signing is configured")
        self.assertLess(block, gate.index("exit 1"))
        self.assertIn(
            "RC/stable release blocked: isolated desktop platform signing is not configured",
            gate,
        )
        self.assertIn("exit 1", gate)
        self.assertNotIn("Block RC and stable", publish)
        self.assertIn("codesign --verify --deep --strict", workflow)
        self.assertIn("spctl --assess --type execute", workflow)
        self.assertIn("xcrun stapler validate", workflow)
        self.assertIn("signtool verify /pa /all /v", workflow)

    def test_release_candidate_and_stable_fail_closed_on_signing_and_publication(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "release.yml").read_text(
            encoding="utf-8"
        )

        gate_start = workflow.index("  release-gate:")
        gate_end = workflow.index("\n  build:", gate_start)
        gate = workflow[gate_start:gate_end]
        sign_start = workflow.index("  sign-native:")
        stage_start = workflow.index("\n  stage-native:", sign_start)
        publish_start = workflow.index("\n  publish:", sign_start)
        signing = workflow[sign_start:stage_start]
        publish = workflow[publish_start:]

        self.assertIn("needs: verify", gate)
        self.assertIn(
            "if: contains(github.ref_name, '-rc.') || !contains(github.ref_name, '-')",
            gate,
        )
        self.assertIn(
            "RC/stable release blocked: isolated desktop platform signing is not configured",
            gate,
        )
        self.assertIn("exit 1", gate)
        self.assertNotIn("environment:", gate)
        self.assertIn("Enforce release approval", signing)
        self.assertIn("environment: release-publication", signing)
        self.assertIn("PANDORA_RELEASE_CANDIDATE_APPROVED:", signing)
        self.assertIn("PANDORA_STABLE_RELEASE_APPROVED:", signing)
        self.assertIn('if [[ "$version" == *-rc.* ]]', signing)
        self.assertIn("PANDORA_WINDOWS_CERTIFICATE_BASE64:", signing)
        self.assertIn("PANDORA_APPLE_CERTIFICATE_BASE64:", signing)
        self.assertIn("APPLE_TEAM_ID:", signing)
        self.assertNotIn("APPLE_ID", signing)
        self.assertIn("needs: [verify, build, release-gate]", signing)
        self.assertIn("needs: [verify, release-gate, stage-native, build-desktop]", publish)
        self.assertNotIn("Block RC and stable", publish)
        self.assertIn("environment: release-publication", workflow)
        self.assertIn("Validate required native accessibility evidence", workflow)
        self.assertIn("scripts/accessibility_evidence.py", workflow)

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
        self.assertIn("Get-AuthenticodeSignature", workflow)
        self.assertIn('codesign --verify --strict --verbose=2 "$native"', workflow)
        self.assertIn("xcrun stapler validate", workflow)
        self.assertIn("spctl --assess --type execute", workflow)
        self.assertIn("platform-signature-verification.json", workflow)

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

    def test_ci_runs_ephemeral_system_installer_lifecycle(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )

        self.assertIn("Verify desktop system install lifecycle", workflow)
        self.assertIn('CI: "true"', workflow)
        self.assertIn('PANDORA_DESKTOP_SYSTEM_INSTALL_LIFECYCLE: "1"', workflow)
        self.assertIn("npm run verify:bundle-lifecycle", workflow)

    def test_ci_retains_exact_commit_unsigned_native_test_packages(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )

        stage = workflow.index("- name: Preserve exact native accessibility test package")
        rollback = workflow.index("- name: Verify desktop install, update, rollback, and uninstall")
        upload = workflow.index("- name: Retain exact native accessibility test package")

        self.assertLess(stage, rollback)
        self.assertLess(rollback, upload)
        self.assertIn("stage-native-test-package.mjs", workflow)
        self.assertIn("github.ref == 'refs/heads/main'", workflow)
        self.assertIn("native-test-package-${{ matrix.platform }}-${{ github.sha }}", workflow)
        self.assertIn("retention-days: 30", workflow[upload:])

    def test_ci_runs_synthetic_desktop_upgrade_rollback_drill(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )

        create = workflow.index("- name: Create synthetic desktop upgrade identities")
        predecessor = workflow.index("- name: Build synthetic predecessor desktop bundle")
        current = workflow.index("- name: Build synthetic current desktop bundle")
        verify = workflow.index(
            "- name: Verify desktop install, update, rollback, and uninstall"
        )
        self.assertLess(create, predecessor)
        self.assertLess(predecessor, current)
        self.assertLess(current, verify)
        self.assertIn("create-upgrade-drill-configs.mjs", workflow)
        self.assertIn("predecessor.json", workflow)
        self.assertIn("current.json", workflow)
        self.assertIn("PANDORA_DESKTOP_UPGRADE_MANIFEST:", workflow)
        self.assertIn("npm run verify:bundle-upgrade-lifecycle", workflow)
