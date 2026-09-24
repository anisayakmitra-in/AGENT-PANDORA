from __future__ import annotations

import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from scripts.agent_pipeline_evidence import (
    EvidenceError,
    create_manifest,
    verify_manifest,
)


class AgentPipelineEvidenceTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="pandora-agent-evidence-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.evidence = self.root / "evidence"
        self.evidence.mkdir()
        (self.evidence / "evaluation.json").write_text(
            '{"command":"evaluation golden","total":1,"passed":1,"failed":0,"digest":"fixture-evaluation"}\n',
            encoding="utf-8",
        )
        (self.evidence / "signed-distribution.log").write_text(
            "authority separation test passed\n", encoding="utf-8"
        )
        (self.evidence / "canary.log").write_text(
            "canary stops before activation test passed\n", encoding="utf-8"
        )
        self.artifact = self.root / "sdk" / "example" / "candidate.wasm"
        self.artifact.parent.mkdir(parents=True)
        artifact_bytes = b"tracked candidate bytes"
        self.artifact.write_bytes(artifact_bytes)
        artifact_digest = "sha256:" + hashlib.sha256(artifact_bytes).hexdigest()
        package_id = "example/candidate"
        package_version = "1.0.0"
        (self.artifact.parent / "pandora.package.json").write_text(
            json.dumps(
                {
                    "id": package_id,
                    "version": package_version,
                    "kind": "gene",
                    "content_hash": artifact_digest,
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        (self.evidence / "package-validation.jsonl").write_text(
            json.dumps(
                {
                    "command": "package validate",
                    "valid": True,
                    "persisted": False,
                    "package": {
                        "id": package_id,
                        "version": package_version,
                        "content_hash": artifact_digest,
                    },
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        subprocess.run(["git", "init", "-q"], cwd=self.root, check=True)
        subprocess.run(
            ["git", "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "--allow-empty", "-q", "-m", "fixture"],
            cwd=self.root,
            check=True,
        )
        subprocess.run(
            ["git", "add", "--", "sdk/example/candidate.wasm", "sdk/example/pandora.package.json"],
            cwd=self.root,
            check=True,
        )
        subprocess.run(
            ["git", "-c", "user.name=Test", "-c", "user.email=test@example.invalid", "commit", "-q", "-m", "add tracked SDK package"],
            cwd=self.root,
            check=True,
        )
        self.commit = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=self.root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()
        self.channel = "beta"
        pipeline = {
            "schema_version": 1,
            "commit": self.commit,
            "channel": self.channel,
            "package_admission_performed": False,
            "signed_distribution_boundary_tested": True,
            "canary_stops_before_activation_tested": True,
            "artifact_activation_performed": False,
            "release_tag_created": False,
            "authority": "evidence_only",
        }
        (self.evidence / "pipeline.json").write_text(
            json.dumps(pipeline, sort_keys=True) + "\n", encoding="utf-8"
        )
        self.manifest_path = self.evidence / "evidence-manifest.json"
        self.manifest = create_manifest(self.evidence, self.commit, self.channel, self.root)
        self.artifact_digest = "sha256:" + hashlib.sha256(self.artifact.read_bytes()).hexdigest()

    def verify(self, **overrides: object) -> None:
        values = {
            "repository_root": self.root,
            "directory": self.evidence,
            "manifest_path": self.manifest_path,
            "commit": self.commit,
            "channel": self.channel,
            "expected_evidence_digest": self.manifest["digest"],
            "artifact_path": "sdk/example/candidate.wasm",
            "expected_artifact_digest": self.artifact_digest,
        }
        values.update(overrides)
        verify_manifest(**values)  # type: ignore[arg-type]

    def test_exact_manifest_and_tracked_artifact_verify(self) -> None:
        self.verify()

    def test_rejects_changed_evidence_after_manifest_creation(self) -> None:
        (self.evidence / "evaluation.json").write_text(
            '{"command":"evaluation golden","total":1,"passed":1,"failed":0,"digest":"tampered-evaluation"}\n',
            encoding="utf-8",
        )
        with self.assertRaisesRegex(EvidenceError, "evidence files changed"):
            self.verify()

    def test_rejects_unrelated_submitted_evidence_digest(self) -> None:
        with self.assertRaisesRegex(EvidenceError, "submitted evidence digest"):
            self.verify(expected_evidence_digest="sha256:" + "0" * 64)

    def test_rejects_artifact_digest_substitution(self) -> None:
        with self.assertRaisesRegex(EvidenceError, "artifact digest does not match"):
            self.verify(expected_artifact_digest="sha256:" + "0" * 64)

    def test_rejects_untracked_artifact(self) -> None:
        extra = self.root / "sdk" / "example" / "untracked.wasm"
        extra.write_bytes(b"not in the source commit")
        with self.assertRaisesRegex(EvidenceError, "not tracked"):
            self.verify(artifact_path="sdk/example/untracked.wasm")

    def test_rejects_tracked_artifact_not_present_in_package_validation_evidence(self) -> None:
        (self.evidence / "package-validation.jsonl").write_text(
            json.dumps(
                {
                    "command": "package validate",
                    "valid": True,
                    "persisted": False,
                    "package": {
                        "id": "example/other",
                        "version": "1.0.0",
                        "content_hash": self.artifact_digest,
                    },
                },
                sort_keys=True,
            )
            + "\n",
            encoding="utf-8",
        )
        self.manifest = create_manifest(self.evidence, self.commit, self.channel, self.root)
        with self.assertRaisesRegex(EvidenceError, "not validated exactly once"):
            self.verify()

    def test_rejects_artifact_worktree_bytes_that_differ_from_the_commit(self) -> None:
        self.artifact.write_bytes(b"changed after commit")
        digest = "sha256:" + hashlib.sha256(self.artifact.read_bytes()).hexdigest()
        with self.assertRaisesRegex(EvidenceError, "working-tree bytes differ"):
            self.verify(expected_artifact_digest=digest)

    def test_rejects_traversing_artifact_path(self) -> None:
        with self.assertRaisesRegex(EvidenceError, "repository-relative"):
            self.verify(artifact_path="../outside.wasm")

    def test_rejects_non_sdk_or_non_package_artifacts(self) -> None:
        with self.assertRaisesRegex(EvidenceError, "SDK .artifact or .wasm"):
            self.verify(artifact_path="README.md")

    def test_rejects_commit_or_channel_substitution(self) -> None:
        with self.assertRaisesRegex(EvidenceError, "checked-out source commit"):
            self.verify(commit="b" * 40)
        with self.assertRaisesRegex(EvidenceError, "identity does not match"):
            self.verify(channel="stable")

    def test_manifest_digest_is_stable_for_identical_evidence(self) -> None:
        first = self.manifest["digest"]
        second = create_manifest(self.evidence, self.commit, self.channel, self.root)["digest"]
        self.assertEqual(first, second)

    def test_command_line_create_and_verify_round_trip(self) -> None:
        script = Path(__file__).with_name("agent_pipeline_evidence.py")
        environment = {
            **os.environ,
            "PATH": f"C:\\Program Files\\Git\\cmd;{os.environ.get('PATH', '')}",
        }
        create = subprocess.run(
            [
                sys.executable,
                str(script),
                "create",
                "--directory",
                str(self.evidence),
                "--repository-root",
                str(self.root),
                "--commit",
                self.commit,
                "--channel",
                self.channel,
            ],
            check=True,
            capture_output=True,
            text=True,
            env=environment,
        )
        self.assertEqual(create.stdout.strip(), self.manifest["digest"])
        subprocess.run(
            [
                sys.executable,
                str(script),
                "verify",
                "--repository-root",
                str(self.root),
                "--directory",
                str(self.evidence),
                "--manifest",
                str(self.manifest_path),
                "--commit",
                self.commit,
                "--channel",
                self.channel,
                "--evidence-digest",
                self.manifest["digest"],
                "--artifact-path",
                "sdk/example/candidate.wasm",
                "--artifact-digest",
                self.artifact_digest,
            ],
            check=True,
            capture_output=True,
            text=True,
            env=environment,
        )

    def test_rejects_failed_package_validation_or_evaluation(self) -> None:
        (self.evidence / "package-validation.jsonl").write_text(
            '{"command":"package validate","valid":false,"persisted":false}\n',
            encoding="utf-8",
        )
        with self.assertRaisesRegex(EvidenceError, "not a successful"):
            create_manifest(self.evidence, self.commit, self.channel, self.root)

        (self.evidence / "package-validation.jsonl").write_text(
            '{"command":"package validate","valid":true,"persisted":false}\n',
            encoding="utf-8",
        )
        (self.evidence / "evaluation.json").write_text(
            '{"command":"evaluation golden","total":1,"passed":0,"failed":1}\n',
            encoding="utf-8",
        )
        with self.assertRaisesRegex(EvidenceError, "complete passing golden-set"):
            create_manifest(self.evidence, self.commit, self.channel, self.root)

    def test_rejects_pipeline_evidence_that_claims_activation(self) -> None:
        pipeline = self.evidence / "pipeline.json"
        content = json.loads(pipeline.read_text(encoding="utf-8"))
        content["artifact_activation_performed"] = True
        pipeline.write_text(json.dumps(content, sort_keys=True) + "\n", encoding="utf-8")
        with self.assertRaisesRegex(EvidenceError, "safe boundary"):
            create_manifest(self.evidence, self.commit, self.channel, self.root)


if __name__ == "__main__":
    unittest.main()
