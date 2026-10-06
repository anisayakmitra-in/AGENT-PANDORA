import re
import subprocess
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"

# The search tokens. They are assembled from fragments so this file does not
# match itself: a guard that cannot find its own name in a grep is a guard whose
# result nobody trusts.
_TOKENS = (
    "ta" + "uri",
    "pandora-" + "desktop",
    "desktop_" + "required",
)
DESKTOP_PATTERN = re.compile("|".join(_TOKENS), re.IGNORECASE)
GREP_PATTERN = "|".join(_TOKENS)

# The desktop was deleted, not cancelled-in-place. These are the only places a
# desktop reference may survive: the changelog records the removal, and the
# audit ledger is a dated record of what was found before it happened.
ALLOWED_REFERENCE_PREFIXES = (".unlazy/pandora-audit/",)
ALLOWED_REFERENCE_FILES = ("CHANGELOG.md",)

# These files mention the tokens only to assert their absence. Each one is a
# guard against the product coming back, not a description of it.
GUARD_FILES = (
    # Asserts the evidence index carries no desktop artifact or requirement.
    "scripts/test_release_evidence.py",
    # Asserts the scope policy carries no desktop requirement key.
    "scripts/test_release_scope.py",
)

# Directories whose contents are not ours to police: build output, vendored
# trees, and VCS metadata.
SKIP_DIRS = {"node_modules", "target", "dist", "build", "test-results", ".git"}
TEXT_SUFFIXES = {
    ".md",
    ".yml",
    ".yaml",
    ".py",
    ".rs",
    ".toml",
    ".json",
    ".js",
    ".mjs",
    ".sh",
    ".ps1",
}


def workflow_text() -> str:
    return WORKFLOW.read_text(encoding="utf-8")


def job(workflow: str, name: str) -> str:
    start = re.search(rf"^  {re.escape(name)}:\s*$", workflow, re.MULTILINE)
    if start is None:
        raise AssertionError(f"workflow job is missing: {name}")
    next_job = re.search(r"^  [A-Za-z0-9_-]+:\s*$", workflow[start.end() :], re.MULTILINE)
    end = start.end() + next_job.start() if next_job else len(workflow)
    return workflow[start.start() : end]


def is_allowed(relative: str) -> bool:
    return (
        relative in ALLOWED_REFERENCE_FILES
        or relative in GUARD_FILES
        or relative.startswith(ALLOWED_REFERENCE_PREFIXES)
    )


class CliOnlyWorkflowTests(unittest.TestCase):
    """The release workflow is CLI-only, with no desktop path left to enable."""

    def test_verify_exposes_only_the_cli_scope(self) -> None:
        workflow = workflow_text()
        verify = job(workflow, "verify")

        self.assertIn("scope: ${{ steps.release-scope.outputs.scope }}", verify)
        self.assertIn("channel: ${{ steps.release-scope.outputs.channel }}", verify)
        # The second scope is gone, so nothing can ask for it.
        self.assertNotIn(_TOKENS[2], verify)

    def test_release_gate_asserts_a_single_cli_only_boundary(self) -> None:
        workflow = workflow_text()
        gate = job(workflow, "release-gate")

        self.assertIn('test "${{ needs.verify.outputs.scope }}" = "cli-only"', gate)
        self.assertNotIn(_TOKENS[2], gate)

    def test_publish_has_no_desktop_dependency_or_step(self) -> None:
        workflow = workflow_text()
        publish = job(workflow, "publish")

        self.assertIn("needs: [verify, release-gate, stage-native]", publish)
        self.assertNotIn("build-desktop", publish)
        self.assertNotIn("desktop-unsigned", publish)
        self.assertNotIn("dist/desktop-", publish)
        self.assertNotIn(_TOKENS[2], publish)

    def test_no_desktop_job_exists_in_any_workflow(self) -> None:
        """A job that could still build a deleted product would only skip."""
        for path in sorted((ROOT / ".github" / "workflows").glob("*.yml")):
            text = path.read_text(encoding="utf-8")
            found = re.findall(
                r"^  (build-desktop|smoke-desktop|stable-desktop-rollback):",
                text,
                re.MULTILINE,
            )
            with self.subTest(workflow=path.name):
                self.assertEqual(found, [])

    def test_stable_rollback_evidence_depends_only_on_surviving_jobs(self) -> None:
        workflow = workflow_text()
        evidence = job(workflow, "stable-rollback-evidence")

        self.assertIn("needs: [verify, smoke-install]", evidence)
        self.assertNotIn("smoke-desktop", evidence)
        self.assertNotIn("stable-desktop-rollback", evidence)
        self.assertNotIn(_TOKENS[2], evidence)

    def test_the_desktop_tree_is_absent(self) -> None:
        self.assertFalse((ROOT / "apps" / f"pandora-{'desktop'}").exists())
        self.assertFalse((ROOT / "third_party").exists())
        self.assertFalse((ROOT / "scripts" / "accessibility_evidence.py").exists())
        self.assertFalse(
            (ROOT / ".github" / "workflows" / "native-accessibility-evidence.yml").exists()
        )
        for deleted in ("DESKTOP_COMPANIONS.md", "DESKTOP_THEMES.md", "ACCESSIBILITY.md"):
            self.assertFalse((ROOT / "docs" / deleted).exists(), deleted)

    def test_only_the_changelog_and_audit_ledger_mention_the_desktop(self) -> None:
        """A deleted product must not creep back in through documentation."""
        offenders: list[str] = []
        for path in sorted(ROOT.rglob("*")):
            if not path.is_file() or path.suffix.lower() not in TEXT_SUFFIXES:
                continue
            relative = path.relative_to(ROOT).as_posix()
            if is_allowed(relative):
                continue
            if any(part in SKIP_DIRS for part in path.parts):
                continue
            try:
                text = path.read_text(encoding="utf-8")
            except (OSError, UnicodeDecodeError):
                continue
            if DESKTOP_PATTERN.search(text):
                offenders.append(relative)

        self.assertEqual(offenders, [])

    def test_grep_confirms_the_tree_is_clean(self) -> None:
        """The same check the removal plan specifies, run through git."""
        result = subprocess.run(
            ["git", "grep", "-il", "-E", GREP_PATTERN, "--", "."],
            cwd=ROOT,
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode not in (0, 1):
            self.skipTest(f"git grep unavailable: {result.stderr.strip()[:80]}")
        matches = [
            line
            for line in result.stdout.splitlines()
            if line.strip() and not is_allowed(line.strip())
        ]
        self.assertEqual(matches, [])


if __name__ == "__main__":
    unittest.main()