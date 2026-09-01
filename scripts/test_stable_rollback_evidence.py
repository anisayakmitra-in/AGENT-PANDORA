from __future__ import annotations

import unittest

from scripts.stable_rollback_evidence import (
    StableRollbackEvidenceError,
    build_stable_rollback_evidence,
)


COMMIT = "27c711c5066dbd59d4834b25565abf7241e7c103"


class StableRollbackEvidenceTests(unittest.TestCase):
    def test_first_stable_records_the_unavoidable_first_patch_gate(self) -> None:
        evidence = build_stable_rollback_evidence(
            "# Changelog\n\n## v2.0.0\n\nFirst stable.\n\n## v2.0.0-rc.1\n\nRC.\n",
            "v2.0.0",
            COMMIT,
            "12345",
        )

        self.assertFalse(evidence["complete"])
        self.assertEqual(evidence["state"], "pending_first_patch")
        self.assertIsNone(evidence["predecessor_tag"])
        self.assertEqual(evidence["closure_release"], "v2.0.1")

    def test_patch_records_exact_compatible_stable_rollback(self) -> None:
        evidence = build_stable_rollback_evidence(
            "# Changelog\n\n## v2.0.2\n\nPatch.\n\n## v2.0.1\n\nPatch.\n\n## v2.0.0\n\nStable.\n",
            "v2.0.2",
            COMMIT,
            "67890",
        )

        self.assertTrue(evidence["complete"])
        self.assertEqual(evidence["state"], "verified")
        self.assertEqual(evidence["predecessor_tag"], "v2.0.1")
        self.assertTrue(evidence["install_update_backup_restore_rollback_uninstall"])

    def test_patch_without_compatible_stable_predecessor_fails_closed(self) -> None:
        with self.assertRaisesRegex(
            StableRollbackEvidenceError, "no compatible stable predecessor"
        ):
            build_stable_rollback_evidence(
                "# Changelog\n\n## v2.0.1\n\nPatch.\n\n## v1.9.9\n\nOld line.\n",
                "v2.0.1",
                COMMIT,
                "12345",
            )

    def test_rejects_prerelease_duplicate_missing_and_unbound_records(self) -> None:
        with self.assertRaisesRegex(StableRollbackEvidenceError, "stable tag"):
            build_stable_rollback_evidence(
                "## v2.0.0-rc.1\n",
                "v2.0.0-rc.1",
                COMMIT,
                "12345",
            )
        with self.assertRaisesRegex(StableRollbackEvidenceError, "duplicate"):
            build_stable_rollback_evidence(
                "## v2.0.0\n\nOne.\n\n## v2.0.0\n\nTwo.\n",
                "v2.0.0",
                COMMIT,
                "12345",
            )
        with self.assertRaisesRegex(StableRollbackEvidenceError, "exact commit"):
            build_stable_rollback_evidence(
                "## v2.0.0\n\nStable.\n",
                "v2.0.0",
                "not-a-sha",
                "12345",
            )
        with self.assertRaisesRegex(StableRollbackEvidenceError, "workflow run ID"):
            build_stable_rollback_evidence(
                "## v2.0.0\n\nStable.\n",
                "v2.0.0",
                COMMIT,
                "0",
            )


if __name__ == "__main__":
    unittest.main()
