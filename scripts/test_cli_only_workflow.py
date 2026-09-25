import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"

# Markdown that may mention the desktop without restating the cancellation.
# Each entry earns its place: a changelog records what happened, vendored and
# third-party trees are not ours to annotate, and .unlazy/ holds agent working
# state rather than shipped documentation.
MARKDOWN_EXEMPT_PREFIXES = (
    ".unlazy/",
    "third_party/",
    "apps/pandora-desktop/public/vendor/",
)
MARKDOWN_EXEMPT_FILES = ("CHANGELOG.md",)
MARKDOWN_SKIP_DIRS = {"node_modules", "target", "dist", "build", "test-results"}

DESKTOP_MENTION = re.compile(r"\bdesktop\b|\btauri\b", re.IGNORECASE)
CANCELLATION_MARKER = re.compile(
    r"\bcancel(?:led|led)?\b|\bretained\b|\binactive\b|\bhistorical\b|\bretired\b",
    re.IGNORECASE,
)


def tracked_markdown() -> list[Path]:
    found: list[Path] = []
    for path in ROOT.rglob("*.md"):
        if any(part in MARKDOWN_SKIP_DIRS for part in path.parts):
            continue
        found.append(path.relative_to(ROOT).as_posix())
    return sorted(found)


def workflow_text() -> str:
    return WORKFLOW.read_text(encoding="utf-8")


def job(workflow: str, name: str) -> str:
    start = re.search(rf"^  {re.escape(name)}:\s*$", workflow, re.MULTILINE)
    if start is None:
        raise AssertionError(f"workflow job is missing: {name}")
    next_job = re.search(r"^  [A-Za-z0-9_-]+:\s*$", workflow[start.end() :], re.MULTILINE)
    end = start.end() + next_job.start() if next_job else len(workflow)
    return workflow[start.start() : end]


class CliOnlyWorkflowTests(unittest.TestCase):
    def test_verify_resolves_and_exposes_source_bound_scope(self) -> None:
        verify = job(workflow_text(), "verify")

        self.assertIn("outputs:", verify)
        self.assertIn("scope: ${{ steps.release-scope.outputs.scope }}", verify)
        self.assertIn("channel: ${{ steps.release-scope.outputs.channel }}", verify)
        self.assertIn(
            "desktop_required: ${{ steps.release-scope.outputs.desktop_required }}",
            verify,
        )
        self.assertIn("id: release-scope", verify)
        self.assertIn(
            'python scripts/release_scope.py "$GITHUB_REF_NAME" --github-output "$GITHUB_OUTPUT"',
            verify,
        )

    def test_cli_only_scope_skips_desktop_build_but_publishes_native_assets(self) -> None:
        workflow = workflow_text()
        desktop = job(workflow, "build-desktop")
        publish = job(workflow, "publish")

        self.assertIn(
            "if: needs.verify.outputs.desktop_required == 'true'", desktop
        )
        self.assertIn("needs: [verify, release-gate, stage-native, build-desktop]", publish)
        self.assertIn("!cancelled()", publish)
        self.assertNotIn("always()", publish)
        self.assertIn("needs.verify.result == 'success'", publish)
        self.assertIn("needs.release-gate.result == 'success'", publish)
        self.assertIn("needs.stage-native.result == 'success'", publish)
        self.assertIn("needs.build-desktop.result == 'success'", publish)
        self.assertIn(
            "needs.build-desktop.result == 'skipped'",
            publish,
        )
        self.assertIn(
            "needs.verify.outputs.scope == 'cli-only'",
            publish,
        )
        self.assertIn(
            "needs.verify.outputs.desktop_required == 'false'",
            publish,
        )
        self.assertIn(
            "if: needs.verify.outputs.desktop_required == 'true'",
            publish[publish.index("- name: Download unsigned desktop artifacts") :],
        )
        self.assertIn(
            'release_evidence.py "$GITHUB_REF_NAME" --dist dist --scope "${{ needs.verify.outputs.scope }}"',
            publish,
        )

    def test_full_scope_requires_desktop_attestation(self) -> None:
        publish = job(workflow_text(), "publish")
        desktop_download = publish.index("- name: Download unsigned desktop artifacts")
        native_attest = publish.index("- name: Attest native artifacts")
        desktop_attest = publish.index("- name: Attest desktop artifacts")
        evidence = publish.index("- name: Generate release evidence index")

        self.assertLess(desktop_download, native_attest)
        self.assertLess(native_attest, desktop_attest)
        self.assertLess(desktop_attest, evidence)
        desktop_step = publish[desktop_attest:evidence]
        self.assertIn(
            "if: needs.verify.outputs.desktop_required == 'true'", desktop_step
        )

    def test_desktop_smoke_and_rollback_jobs_only_run_for_full_scope(self) -> None:
        workflow = workflow_text()
        smoke = job(workflow, "smoke-desktop")
        rollback = job(workflow, "stable-desktop-rollback")
        evidence = job(workflow, "stable-rollback-evidence")

        for block in (smoke, rollback, evidence):
            self.assertRegex(block, r"(?m)^    needs:.*\bverify\b")
            self.assertIn("needs.verify.outputs.desktop_required == 'true'", block)
        self.assertIn("needs: [publish, verify]", smoke)
        self.assertIn("needs: [publish, verify]", rollback)
        self.assertIn(
            "needs: [verify, smoke-install, smoke-desktop, stable-desktop-rollback]",
            evidence,
        )

    def test_documentation_describes_source_bound_scope_without_desktop_overclaim(self) -> None:
        production = (ROOT / "docs" / "PRODUCTION.md").read_text(encoding="utf-8")
        releases = (ROOT / "RELEASES.md").read_text(encoding="utf-8")

        for document in (production, releases):
            self.assertIn("release-scope.json", document)
            self.assertIn("CLI-only", document)
            self.assertIn("full", document)
        self.assertIn("does not claim desktop parity", releases)
        self.assertIn("must not contain desktop artifacts", production)

    def test_every_markdown_mentioning_the_desktop_states_that_it_is_cancelled(self) -> None:
        unmarked: list[str] = []
        for relative in tracked_markdown():
            if relative in MARKDOWN_EXEMPT_FILES:
                continue
            if relative.startswith(MARKDOWN_EXEMPT_PREFIXES):
                continue
            text = (ROOT / relative).read_text(encoding="utf-8")
            if not DESKTOP_MENTION.search(text):
                continue
            if not CANCELLATION_MARKER.search(text):
                unmarked.append(relative)
        self.assertEqual(
            unmarked,
            [],
            "markdown mentions the desktop without stating it is cancelled: "
            + ", ".join(unmarked),
        )


if __name__ == "__main__":
    unittest.main()
