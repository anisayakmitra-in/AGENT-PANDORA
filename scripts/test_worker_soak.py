from __future__ import annotations

import unittest

from scripts.worker_soak import (
    PLATFORMS,
    WorkerSoakError,
    build_campaign,
    profile_segments,
    render_campaign_markdown,
    validate_segment,
)


def passing_segment(platform: str, segment: int, duration: int) -> dict[str, object]:
    return {
        "schema_version": 1,
        "status": "passed",
        "commit": "a" * 40,
        "platform": platform,
        "profile": "eight-hour",
        "segment": segment,
        "segment_count": 2,
        "configuration": {
            "duration_seconds": duration,
            "jobs": 512,
            "producers": 4,
            "rounds": 1,
        },
        "checks": {
            "cancellation_race": {"status": "passed", "exit_code": 0},
            "worker_operations": {"status": "passed", "exit_code": 0},
        },
        "runtime": {
            "status": "passed",
            "elapsed_seconds": duration + 1,
            "configuration": {
                "producers": 4,
                "rounds": 1,
                "recovery_spread_seconds": duration,
            },
            "gates": {
                "all_jobs_completed": True,
                "exactly_once": True,
                "no_active_leases": True,
                "no_running_supervisors": True,
                "resource_samples_present": True,
                "stale_supervisor_observed": True,
                "state_sampling_reliable": True,
                "memory_growth_within_limit": True,
                "clean_restart_and_shutdown": True,
                "partial_multi_repository_failure_preserved": True,
            },
            "outcomes": {
                "total_jobs": 512,
                "completed_jobs": 512,
                "unique_sessions": 512,
                "unique_executions": 512,
                "unique_effect_receipts": 512,
            },
            "metrics": {
                "max_queue_depth": 7,
                "peak_rss_bytes": 12_345,
                "max_cpu_percent": 42.5,
                "max_active_lease_age_seconds": 11,
            },
        },
    }


class WorkerSoakTests(unittest.TestCase):
    def test_profiles_have_exact_expected_elapsed_time(self) -> None:
        self.assertEqual(profile_segments("two-hour"), (7_200,))
        self.assertEqual(sum(profile_segments("eight-hour")), 28_800)
        self.assertEqual(sum(profile_segments("twenty-four-hour")), 86_400)

    def test_segment_validation_rejects_mislabeled_duration(self) -> None:
        with self.assertRaisesRegex(WorkerSoakError, "must run for 7200 seconds"):
            validate_segment("two-hour", 1, 600, 512, 4, 1, "linux-x64")

    def test_campaign_requires_every_platform_and_segment(self) -> None:
        items = [
            passing_segment(platform, segment, 14_400)
            for platform in PLATFORMS
            for segment in (1, 2)
        ]
        campaign = build_campaign("eight-hour", "a" * 40, items)

        self.assertEqual(campaign["status"], "passed")
        self.assertEqual(campaign["requested_seconds_per_platform"], 28_800)
        self.assertEqual(campaign["platforms"]["linux-x64"]["total_jobs"], 1_024)
        self.assertIn("Status: **passed**", render_campaign_markdown(campaign))

        incomplete = build_campaign("eight-hour", "a" * 40, items[:-1])
        self.assertEqual(incomplete["status"], "failed")
        self.assertIn("missing evidence", " ".join(incomplete["errors"]))

    def test_campaign_fails_closed_on_runtime_gate(self) -> None:
        items = [
            passing_segment(platform, segment, 14_400)
            for platform in PLATFORMS
            for segment in (1, 2)
        ]
        items[0]["runtime"]["gates"]["no_active_leases"] = False

        campaign = build_campaign("eight-hour", "a" * 40, items)

        self.assertEqual(campaign["status"], "failed")
        self.assertIn("failed gate no_active_leases", " ".join(campaign["errors"]))


if __name__ == "__main__":
    unittest.main()
