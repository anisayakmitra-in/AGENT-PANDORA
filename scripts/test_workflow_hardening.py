"""Guard the CI hardening properties, using only the standard library.

The repository's other validators avoid third-party packages so they can run in
CI without an extra install step, and this one is no different. It is textual
rather than a full YAML parse: a general parser would need PyYAML, and the
properties worth protecting here are exact, greppable lines.

A workflow that parses but silently lost its timeout, or that grew an unpinned
cache action, is precisely the regression this is meant to catch.
"""

from __future__ import annotations

import json
import os
import re
import unittest
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
WORKFLOWS = ROOT / ".github" / "workflows"

# The live policy that decides which actions a workflow may reference.
OWNER = "anisayakmitra-in"
REPO = "AGENT-PANDORA"
API = "https://api.github.com"

# Actions published by GitHub itself, covered by `github_owned_allowed`.
GITHUB_OWNED_OWNERS = frozenset({"actions", "github"})

# Every job that can actually run, as (job key, declared name). A hung job
# otherwise runs to the 360-minute platform default. The name is asserted too,
# so renaming a job cannot quietly move it out of this table.
JOBS_REQUIRING_TIMEOUT = {
    "ci.yml": (
        ("verify", "Verify (${{ matrix.os }})"),
        ("desktop", "Desktop (${{ matrix.platform }})"),
    ),
    "security.yml": (("audit", "Dependency and repository audit"),),
    "codeql.yml": (("analyze", "Analyze Rust"),),
    "agent-pipeline.yml": (
        ("validate", "Admission, evaluation, and canary evidence"),
        ("promotion-gate", "Approved tag creation (${{ inputs.channel }})"),
    ),
    # release.yml defines eleven jobs; the reachable ones are asserted
    # individually and the whole spine is swept by the test below.
    "release.yml": (
        ("verify", "Verify release source"),
        ("release-gate", "Gate protected release jobs"),
    ),
}

# The reviewed rust-cache pin. Changing it is a supply-chain decision, so the
# exact commit is asserted rather than any v2 tag.
RUST_CACHE_PIN = "Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6"
WORKFLOWS_REQUIRING_CACHE = ("ci.yml", "security.yml", "fuzz.yml")

# A workflow with no rust-cache step must carry this marker, followed by a
# colon and the reason, so a removed cache is always a recorded decision.
CACHE_WAIVER_MARKER = "pandora-ci-cache-waived"

# A matrix exists to surface platform differences, so one platform failing must
# not suppress the verdict from the others.
VERIFY_MATRIX = ("ci.yml", "verify")
VERIFY_PLATFORMS = ("ubuntu-latest", "macos-26", "windows-latest")


def workflow(name: str) -> str:
    return (WORKFLOWS / name).read_text(encoding="utf-8")


def job_block(text: str, job_key: str) -> str:
    """Return the body of a job, from its key to the next job key."""
    start = re.search(rf"^  {re.escape(job_key)}:\s*$", text, re.MULTILINE)
    if start is None:
        raise AssertionError(f"workflow job is missing: {job_key}")
    rest = text[start.end() :]
    following = re.search(r"^  [A-Za-z0-9_-]+:\s*$", rest, re.MULTILINE)
    return rest[: following.start()] if following else rest


class WorkflowHardening(unittest.TestCase):
    def test_reachable_jobs_declare_a_timeout(self) -> None:
        for name, jobs in JOBS_REQUIRING_TIMEOUT.items():
            text = workflow(name)
            for job_key, job_name in jobs:
                with self.subTest(workflow=name, job=job_key):
                    body = job_block(text, job_key)
                    self.assertIn(
                        f"name: {job_name}",
                        body,
                        f"{name}: job '{job_key}' was renamed, so this table no "
                        f"longer describes it",
                    )
                    self.assertRegex(
                        body,
                        r"(?m)^\s+timeout-minutes:\s*\d+\s*$",
                        f"{name}: job '{job_name}' has no timeout-minutes, so a hang "
                        f"runs to the 360-minute platform default",
                    )

    def test_release_jobs_that_can_run_declare_a_timeout(self) -> None:
        text = workflow("release.yml")
        keys = re.findall(r"^  ([A-Za-z0-9_-]+):\s*$", text, re.MULTILINE)[1:]
        unreachable = {
            "build-desktop",
            "smoke-desktop",
            "stable-desktop-rollback",
            "stable-rollback-evidence",
        }
        reachable = [key for key in keys if key not in unreachable]
        self.assertGreaterEqual(len(reachable), 7, "expected the release spine to be present")
        for key in reachable:
            with self.subTest(job=key):
                self.assertRegex(
                    job_block(text, key),
                    r"(?m)^\s+timeout-minutes:\s*\d+\s*$",
                    f"release.yml: reachable job '{key}' has no timeout-minutes",
                )

    def test_rust_cache_is_present_and_pinned_to_the_reviewed_commit(self) -> None:
        """Caching is either pinned on, or waived in-file with a recorded reason.

        This used to assert the step unconditionally, which left no way to
        express "we deliberately turned this off". That is a bad gate: it makes
        a temporary, documented removal indistinguishable from an accidental
        one. So absence is now allowed, but only against an explicit marker
        naming the reason.
        """
        for name in WORKFLOWS_REQUIRING_CACHE:
            with self.subTest(workflow=name):
                text = workflow(name)
                pins = re.findall(r"uses:\s*(Swatinem/rust-cache@\S+)", text)
                if pins:
                    for pin in pins:
                        self.assertEqual(
                            pin,
                            RUST_CACHE_PIN,
                            f"{name}: rust-cache must stay pinned to the reviewed commit",
                        )
                    continue
                self.assertIn(
                    CACHE_WAIVER_MARKER,
                    text,
                    f"{name}: no rust-cache step and no recorded reason. Every cargo "
                    f"pass was rebuilding cold, so add the pinned step back or record "
                    f"the '{CACHE_WAIVER_MARKER}' marker with why it is absent",
                )

    def test_pins_are_commit_objects_not_tag_objects(self) -> None:
        """A 40-hex value can still be unusable.

        The first rust-cache pin came from `git ls-remote refs/tags/...` after the
        output was truncated to its last few lines, which kept the annotated tag
        object and dropped the peeled `^{}` commit. Both are 40 hex characters,
        so the shape assertions above accepted it and all three workflows failed
        to start.

        Distinguishing them needs the upstream ref, so this test is opt-in and
        only runs when a token is available. Without it the failure mode is a red
        run rather than a silent wrong answer, which is the acceptable direction.
        """
        if os.environ.get("GITHUB_TOKEN") is None and os.environ.get("GH_TOKEN") is None:
            self.skipTest("no token: verifying a ref resolves to a commit needs the API")
        for pin in sorted(self._all_pins()):
            with self.subTest(pin=pin):
                owner_repo, ref = pin.rsplit("@", 1)
                request = urllib.request.Request(
                    f"https://api.github.com/repos/{owner_repo}/commits/{ref}",
                    headers={"Authorization": f"Bearer {self._token()}"},
                )
                with urllib.request.urlopen(request, timeout=30) as response:
                    self.assertEqual(response.status, 200, f"{pin} does not resolve to a commit")

    def test_the_merge_gate_is_a_single_stable_non_matrix_job(self) -> None:
        """The required check must not embed a runner label or a matrix value.

        Required status checks are matched by name. Pinning the `verify` matrix
        jobs directly would pin three names containing `ubuntu-latest`,
        `macos-26`, and `windows-latest`, so a routine runner bump would block
        merges for a reason unrelated to correctness. The gate job exists to keep
        exactly one stable, matrix-free name in the required set, and it must
        depend on the whole matrix so a platform failure still fails the gate.
        """
        body = job_block(workflow("ci.yml"), "gate")
        self.assertIn(
            "name: CI gate",
            body,
            "the merge gate must keep the exact name 'CI gate', because that is "
            "what branch protection requires",
        )
        self.assertNotRegex(
            body,
            r"(?m)^\s+matrix:",
            "the merge gate must not declare a matrix, or its required check name "
            "would change per platform",
        )
        self.assertRegex(
            body,
            r"(?m)^    needs:\s*\[verify\]\s*$",
            "the merge gate must need the verify matrix, or a platform failure "
            "would not block the merge",
        )
        self.assertRegex(
            body,
            r"(?m)^\s+timeout-minutes:\s*\d+\s*$",
            "the merge gate declares no timeout, so a hung runner would stall merges",
        )

    def test_every_action_is_covered_by_the_selected_actions_policy(self) -> None:
        """A disallowed action fails the run at dispatch, not inside a job.

        The repository runs `allowed_actions: selected`. Referencing an action
        outside that list makes GitHub refuse the whole run: the conclusion is
        `startup_failure`, no job is ever created, the check suite has zero
        check runs, and the REST API reports no reason at all. That is a silent,
        total loss of the pipeline verdict, and it is exactly what the rust-cache
        step did from `dfe6f22` until this was found.

        So the invariant is: every `uses:` reference must be covered by the live
        policy. That needs the API, so this test is opt-in on a token.
        """
        token = self._token()
        if not token:
            self.skipTest("no token: reading the selected-actions policy needs the API")
        request = urllib.request.Request(
            f"{API}/repos/{OWNER}/{REPO}/actions/permissions/selected-actions",
            headers={"Authorization": f"Bearer {token}", "Accept": "application/vnd.github+json"},
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                policy = json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as error:
            # The default CI token is a `contents: read` token and is refused
            # here. Skipping is the honest outcome: the workflow is not broken,
            # this test simply could not ask the question.
            self.skipTest(f"cannot read the selected-actions policy: HTTP {error.code}")

        allowed = set(policy.get("patterns_allowed") or [])
        github_owned = bool(policy.get("github_owned_allowed"))
        verified = bool(policy.get("verified_allowed"))

        for pin in sorted(self._all_action_refs()):
            owner_repo, _ref = pin.rsplit("@", 1)
            owner = owner_repo.split("/", 1)[0]
            with self.subTest(action=pin):
                if owner in GITHUB_OWNED_OWNERS:
                    self.assertTrue(
                        github_owned,
                        f"{pin} is a GitHub-owned action but github_owned_allowed "
                        f"is false, so no workflow using it can dispatch",
                    )
                    continue
                if verified:
                    continue
                self.assertIn(
                    f"{owner_repo}@*",
                    allowed,
                    f"{pin} is not covered by the selected-actions policy. Adding a "
                    f"step that uses it makes every run of that workflow report "
                    f"startup_failure with no jobs and no log. Add '{owner_repo}@*' "
                    f"to the repository's allowed actions, or drop the step.",
                )

    @staticmethod
    def _token() -> str:
        return os.environ.get("GITHUB_TOKEN") or os.environ.get("GH_TOKEN") or ""

    @staticmethod
    def _all_pins() -> set[str]:
        pins: set[str] = set()
        for path in WORKFLOWS.glob("*.yml"):
            pins.update(
                re.findall(
                    r"uses:\s*([\w.-]+/[\w.-]+@[0-9a-f]{40})",
                    path.read_text(encoding="utf-8"),
                )
            )
        return pins

    @staticmethod
    def _all_action_refs() -> set[str]:
        """Every `uses:` action reference, pinned or not.

        The policy matches on `owner/repo`, so an unpinned or locally defined
        action still has to be covered. `uses: ./...` is a local reusable
        workflow and is excluded because the regex requires an owner segment.
        """
        refs: set[str] = set()
        for path in WORKFLOWS.glob("*.yml"):
            refs.update(
                re.findall(
                    r"uses:\s*([\w.-]+/[\w.-]+@[^\s#]+)",
                    path.read_text(encoding="utf-8"),
                )
            )
        return refs

    def test_verify_matrix_reports_every_platform(self) -> None:
        body = job_block(workflow("ci.yml"), VERIFY_MATRIX[1])
        self.assertRegex(
            body,
            r"(?m)^\s+fail-fast:\s*false\s*$",
            "the verify matrix must not use fail-fast, or one platform's failure "
            "cancels the others and hides whether they passed",
        )
        for platform in VERIFY_PLATFORMS:
            self.assertIn(platform, body, f"verify matrix no longer covers {platform}")

    def test_no_workflow_uses_a_floating_action_tag(self) -> None:
        for path in sorted(WORKFLOWS.glob("*.yml")):
            text = path.read_text(encoding="utf-8")
            for line in text.splitlines():
                if "uses:" not in line:
                    continue
                pin = line.split("uses:", 1)[1].strip()
                if pin.startswith("./"):
                    # A local reusable workflow is referenced by path and is
                    # already bound to the commit under test, so there is no
                    # third-party ref to pin.
                    with self.subTest(workflow=path.name, pin=pin):
                        self.assertTrue(
                            pin.startswith("./.github/workflows/"),
                            f"{path.name}: a local workflow call must stay inside .github/workflows",
                        )
                    continue
                ref = pin.split("@", 1)[1].split()[0] if "@" in pin else ""
                with self.subTest(workflow=path.name, pin=pin):
                    self.assertRegex(
                        ref,
                        r"^[0-9a-f]{40}$",
                        "every third-party action must be pinned to a full commit SHA",
                    )


if __name__ == "__main__":
    unittest.main()
