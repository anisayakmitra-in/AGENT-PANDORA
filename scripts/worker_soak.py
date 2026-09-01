from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import subprocess
import sys
from pathlib import Path
from typing import IO, Any, Iterable


PLATFORMS = ("linux-x64", "macos-x64", "macos-arm64", "windows-x64")
PROFILE_SEGMENTS: dict[str, tuple[int, ...]] = {
    "ten-minute": (600,),
    "two-hour": (7_200,),
    # GitHub-hosted jobs have a six-hour execution limit. Longer campaigns are
    # intentionally checkpointed into four-hour jobs with a fresh runner token.
    "eight-hour": (14_400, 14_400),
    "twenty-four-hour": (14_400, 14_400, 14_400, 14_400, 14_400, 14_400),
}


class WorkerSoakError(ValueError):
    pass


def profile_segments(profile: str) -> tuple[int, ...]:
    try:
        return PROFILE_SEGMENTS[profile]
    except KeyError as error:
        choices = ", ".join(PROFILE_SEGMENTS)
        raise WorkerSoakError(f"unknown worker soak profile {profile!r}; choose {choices}") from error


def validate_segment(
    profile: str,
    segment: int,
    duration_seconds: int,
    jobs: int,
    producers: int,
    rounds: int,
    platform: str,
) -> None:
    segments = profile_segments(profile)
    if segment < 1 or segment > len(segments):
        raise WorkerSoakError(
            f"segment must be between 1 and {len(segments)} for {profile}"
        )
    expected_duration = segments[segment - 1]
    if duration_seconds != expected_duration:
        raise WorkerSoakError(
            f"segment {segment} of {profile} must run for {expected_duration} seconds"
        )
    if producers < 2 or producers > 8:
        raise WorkerSoakError("producers must be between 2 and 8")
    if jobs < producers * 4 or jobs > 4_096:
        raise WorkerSoakError(
            f"jobs must be between {producers * 4} and 4096 for {producers} producers"
        )
    if rounds < 1 or rounds > 16:
        raise WorkerSoakError("rounds must be between 1 and 16")
    if platform not in PLATFORMS:
        raise WorkerSoakError(f"unsupported worker soak platform: {platform}")


def _utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat().replace("+00:00", "Z")


def _write_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _as_int(value: object) -> int:
    return value if isinstance(value, int) and not isinstance(value, bool) else 0


def _as_float(value: object) -> float:
    return float(value) if isinstance(value, (int, float)) and not isinstance(value, bool) else 0.0


def _stream_command(command: list[str], log: IO[str], environment: dict[str, str]) -> int:
    rendered = " ".join(command)
    heading = f"\n$ {rendered}\n"
    print(heading, end="", flush=True)
    log.write(heading)
    log.flush()
    process = subprocess.Popen(
        command,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        encoding="utf-8",
        errors="replace",
        bufsize=1,
    )
    assert process.stdout is not None
    for line in process.stdout:
        print(line, end="", flush=True)
        log.write(line)
        log.flush()
    return process.wait()


def _git_commit() -> str:
    configured = os.environ.get("GITHUB_SHA", "").strip()
    if configured:
        return configured
    result = subprocess.run(
        ["git", "rev-parse", "HEAD"],
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    return result.stdout.strip()


def run_segment(arguments: argparse.Namespace) -> int:
    try:
        validate_segment(
            arguments.profile,
            arguments.segment,
            arguments.duration_seconds,
            arguments.jobs,
            arguments.producers,
            arguments.rounds,
            arguments.platform,
        )
    except WorkerSoakError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    started_at = _utc_now()
    runtime_evidence = arguments.output.with_suffix(".runtime.json")
    runtime_evidence.unlink(missing_ok=True)
    arguments.output.unlink(missing_ok=True)
    environment = dict(os.environ)
    environment.update(
        {
            "PANDORA_PHASE7_SOAK": "1",
            "PANDORA_PHASE7_SOAK_SECONDS": str(arguments.duration_seconds),
            "PANDORA_PHASE7_SOAK_JOBS": str(arguments.jobs),
            "PANDORA_PHASE7_SOAK_PRODUCERS": str(arguments.producers),
            "PANDORA_PHASE7_SOAK_ROUNDS": str(arguments.rounds),
            "PANDORA_PHASE7_SOAK_EVIDENCE_PATH": str(runtime_evidence.resolve()),
        }
    )
    cancellation_command = [
        "cargo",
        "test",
        "-p",
        "pandora-cli",
        "--test",
        "cli_smoke",
        "cancellation_during_provider_return_survives_worker_restart_without_replay",
        "--locked",
        "--",
        "--exact",
        "--nocapture",
    ]
    worker_command = [
        "cargo",
        "test",
        "-p",
        "pandora-cli",
        "--test",
        "cli_smoke",
        "phase7_worker_operations_recover_without_replaying_durable_effects",
        "--locked",
        "--",
        "--exact",
        "--nocapture",
    ]

    cancellation_exit: int | None = None
    worker_exit: int | None = None
    runtime: dict[str, Any] | None = None
    error_message: str | None = None
    arguments.log.parent.mkdir(parents=True, exist_ok=True)
    try:
        with arguments.log.open("w", encoding="utf-8", newline="\n") as log:
            cancellation_exit = _stream_command(cancellation_command, log, environment)
            if cancellation_exit == 0:
                worker_exit = _stream_command(worker_command, log, environment)
            else:
                message = "worker operations skipped because cancellation-race evidence failed\n"
                print(message, end="", flush=True)
                log.write(message)
    except (OSError, subprocess.SubprocessError) as error:
        error_message = str(error)

    if runtime_evidence.is_file():
        try:
            loaded = json.loads(runtime_evidence.read_text(encoding="utf-8"))
            if not isinstance(loaded, dict):
                raise WorkerSoakError("runtime evidence must be a JSON object")
            runtime = loaded
        except (OSError, json.JSONDecodeError, WorkerSoakError) as error:
            error_message = f"invalid runtime evidence: {error}"

    runtime_passed = bool(runtime and runtime.get("status") == "passed")
    passed = (
        cancellation_exit == 0
        and worker_exit == 0
        and runtime_passed
        and error_message is None
    )
    evidence: dict[str, Any] = {
        "schema_version": 1,
        "status": "passed" if passed else "failed",
        "commit": _git_commit(),
        "platform": arguments.platform,
        "profile": arguments.profile,
        "segment": arguments.segment,
        "segment_count": len(profile_segments(arguments.profile)),
        "started_at": started_at,
        "finished_at": _utc_now(),
        "configuration": {
            "duration_seconds": arguments.duration_seconds,
            "jobs": arguments.jobs,
            "producers": arguments.producers,
            "rounds": arguments.rounds,
        },
        "checks": {
            "cancellation_race": {
                "status": "passed" if cancellation_exit == 0 else "failed",
                "exit_code": cancellation_exit,
            },
            "worker_operations": {
                "status": "passed" if worker_exit == 0 else "failed",
                "exit_code": worker_exit,
            },
        },
        "runtime": runtime,
        "error": error_message,
    }
    _write_json(arguments.output, evidence)
    print(f"worker soak evidence written to {arguments.output}")
    return 0 if passed else 1


def _candidate_evidence(paths: Iterable[Path]) -> list[dict[str, Any]]:
    evidence: list[dict[str, Any]] = []
    for path in sorted(paths):
        try:
            item = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise WorkerSoakError(f"invalid evidence file {path}: {error}") from error
        if not isinstance(item, dict) or item.get("schema_version") != 1:
            raise WorkerSoakError(f"unsupported evidence file {path}")
        if "platform" in item and "segment" in item:
            evidence.append(item)
    return evidence


def build_campaign(
    profile: str,
    commit: str,
    evidence_items: list[dict[str, Any]],
) -> dict[str, Any]:
    durations = profile_segments(profile)
    expected = {(platform, index + 1) for platform in PLATFORMS for index in range(len(durations))}
    observed: dict[tuple[str, int], dict[str, Any]] = {}
    errors: list[str] = []

    for item in evidence_items:
        platform = item.get("platform")
        segment = item.get("segment")
        key = (platform, segment)
        if platform not in PLATFORMS or not isinstance(segment, int):
            errors.append(f"invalid platform or segment in evidence: {key}")
            continue
        if key in observed:
            errors.append(f"duplicate evidence for {platform} segment {segment}")
            continue
        observed[key] = item

    missing = sorted(expected - set(observed))
    unexpected = sorted(set(observed) - expected)
    for platform, segment in missing:
        errors.append(f"missing evidence for {platform} segment {segment}")
    for platform, segment in unexpected:
        errors.append(f"unexpected evidence for {platform} segment {segment}")

    platform_summaries: dict[str, Any] = {}
    for platform in PLATFORMS:
        requested_seconds = 0
        observed_elapsed_seconds = 0.0
        total_jobs = 0
        max_queue_depth = 0
        peak_rss_bytes = 0
        max_cpu_percent = 0.0
        max_active_lease_age_seconds = 0
        for segment, expected_duration in enumerate(durations, start=1):
            item = observed.get((platform, segment))
            if item is None:
                continue
            if item.get("profile") != profile:
                errors.append(f"{platform} segment {segment} has the wrong profile")
            if item.get("commit") != commit:
                errors.append(f"{platform} segment {segment} has the wrong commit")
            if item.get("status") != "passed":
                errors.append(f"{platform} segment {segment} did not pass")
            if item.get("segment_count") != len(durations):
                errors.append(f"{platform} segment {segment} has the wrong segment count")
            checks = item.get("checks") or {}
            for check in ("cancellation_race", "worker_operations"):
                if (checks.get(check) or {}).get("status") != "passed":
                    errors.append(f"{platform} segment {segment} failed check {check}")
            configuration = item.get("configuration") or {}
            if configuration.get("duration_seconds") != expected_duration:
                errors.append(f"{platform} segment {segment} has the wrong duration")
            runtime = item.get("runtime") or {}
            if runtime.get("status") != "passed":
                errors.append(f"{platform} segment {segment} lacks passing runtime evidence")
            gates = runtime.get("gates") or {}
            for gate in (
                "all_jobs_completed",
                "exactly_once",
                "no_active_leases",
                "no_running_supervisors",
                "resource_samples_present",
                "stale_supervisor_observed",
                "state_sampling_reliable",
                "memory_growth_within_limit",
                "clean_restart_and_shutdown",
                "partial_multi_repository_failure_preserved",
            ):
                if gates.get(gate) is not True:
                    errors.append(f"{platform} segment {segment} failed gate {gate}")
            metrics = runtime.get("metrics") or {}
            outcomes = runtime.get("outcomes") or {}
            runtime_configuration = runtime.get("configuration") or {}
            if runtime_configuration.get("producers") != configuration.get("producers"):
                errors.append(f"{platform} segment {segment} has inconsistent producers")
            if runtime_configuration.get("rounds") != configuration.get("rounds"):
                errors.append(f"{platform} segment {segment} has inconsistent recovery rounds")
            if runtime_configuration.get("recovery_spread_seconds") != expected_duration:
                errors.append(f"{platform} segment {segment} has inconsistent elapsed profile")
            producers = _as_int(configuration.get("producers"))
            rounds = _as_int(configuration.get("rounds"))
            configured_jobs = _as_int(configuration.get("jobs"))
            expected_jobs = producers * 2 + (configured_jobs - producers * 2) * rounds
            for outcome in (
                "total_jobs",
                "completed_jobs",
                "unique_sessions",
                "unique_executions",
                "unique_effect_receipts",
            ):
                if outcomes.get(outcome) != expected_jobs:
                    errors.append(
                        f"{platform} segment {segment} has inconsistent {outcome}"
                    )
            elapsed_seconds = _as_float(runtime.get("elapsed_seconds"))
            if elapsed_seconds < expected_duration:
                errors.append(f"{platform} segment {segment} ended before its requested duration")
            requested_seconds += expected_duration
            observed_elapsed_seconds += elapsed_seconds
            total_jobs += _as_int(outcomes.get("total_jobs"))
            max_queue_depth = max(max_queue_depth, _as_int(metrics.get("max_queue_depth")))
            peak_rss_bytes = max(peak_rss_bytes, _as_int(metrics.get("peak_rss_bytes")))
            max_cpu_percent = max(
                max_cpu_percent, _as_float(metrics.get("max_cpu_percent"))
            )
            max_active_lease_age_seconds = max(
                max_active_lease_age_seconds,
                _as_int(metrics.get("max_active_lease_age_seconds")),
            )
        platform_summaries[platform] = {
            "segments": len(durations),
            "requested_seconds": requested_seconds,
            "observed_elapsed_seconds": round(observed_elapsed_seconds, 3),
            "total_jobs": total_jobs,
            "max_queue_depth": max_queue_depth,
            "peak_rss_bytes": peak_rss_bytes,
            "max_cpu_percent": round(max_cpu_percent, 3),
            "max_active_lease_age_seconds": max_active_lease_age_seconds,
        }

    return {
        "schema_version": 1,
        "status": "passed" if not errors else "failed",
        "profile": profile,
        "commit": commit,
        "generated_at": _utc_now(),
        "segment_count": len(durations),
        "requested_seconds_per_platform": sum(durations),
        "platforms": platform_summaries,
        "errors": errors,
    }


def render_campaign_markdown(campaign: dict[str, Any]) -> str:
    lines = [
        "## Worker soak campaign",
        "",
        f"Status: **{campaign['status']}**  ",
        f"Profile: `{campaign['profile']}`  ",
        f"Commit: `{campaign['commit']}`",
        "",
        "| Platform | Requested | Observed | Jobs | Peak queue | Peak RSS | Max CPU | Max lease age |",
        "|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for platform, summary in campaign["platforms"].items():
        lines.append(
            "| {platform} | {requested}s | {observed:.1f}s | {jobs} | {queue} | "
            "{rss} | {cpu:.1f}% | {lease}s |".format(
                platform=platform,
                requested=summary["requested_seconds"],
                observed=summary["observed_elapsed_seconds"],
                jobs=summary["total_jobs"],
                queue=summary["max_queue_depth"],
                rss=summary["peak_rss_bytes"],
                cpu=summary["max_cpu_percent"],
                lease=summary["max_active_lease_age_seconds"],
            )
        )
    if campaign["errors"]:
        lines.extend(["", "### Evidence errors", ""])
        lines.extend(f"- {error}" for error in campaign["errors"])
    return "\n".join(lines) + "\n"


def aggregate(arguments: argparse.Namespace) -> int:
    try:
        items = _candidate_evidence(arguments.evidence_root.rglob("*.json"))
        campaign = build_campaign(arguments.profile, arguments.commit, items)
        _write_json(arguments.output, campaign)
        markdown = render_campaign_markdown(campaign)
        arguments.markdown_output.parent.mkdir(parents=True, exist_ok=True)
        arguments.markdown_output.write_text(markdown, encoding="utf-8")
        summary_path = os.environ.get("GITHUB_STEP_SUMMARY")
        if summary_path:
            with Path(summary_path).open("a", encoding="utf-8", newline="\n") as summary:
                summary.write(markdown)
    except (OSError, WorkerSoakError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"worker soak campaign evidence written to {arguments.output}")
    return 0 if campaign["status"] == "passed" else 1


def main() -> int:
    parser = argparse.ArgumentParser(description="Run and aggregate worker soak evidence")
    commands = parser.add_subparsers(dest="command", required=True)

    profile_parser = commands.add_parser("profile", help="print a profile definition")
    profile_parser.add_argument("name", choices=tuple(PROFILE_SEGMENTS))

    run_parser = commands.add_parser("run-segment", help="run one platform segment")
    run_parser.add_argument("--profile", required=True, choices=tuple(PROFILE_SEGMENTS))
    run_parser.add_argument("--segment", required=True, type=int)
    run_parser.add_argument("--duration-seconds", required=True, type=int)
    run_parser.add_argument("--jobs", required=True, type=int)
    run_parser.add_argument("--producers", required=True, type=int)
    run_parser.add_argument("--rounds", required=True, type=int)
    run_parser.add_argument("--platform", required=True, choices=PLATFORMS)
    run_parser.add_argument("--log", required=True, type=Path)
    run_parser.add_argument("--output", required=True, type=Path)

    aggregate_parser = commands.add_parser("aggregate", help="build campaign evidence")
    aggregate_parser.add_argument("--profile", required=True, choices=tuple(PROFILE_SEGMENTS))
    aggregate_parser.add_argument("--commit", required=True)
    aggregate_parser.add_argument("--evidence-root", required=True, type=Path)
    aggregate_parser.add_argument("--output", required=True, type=Path)
    aggregate_parser.add_argument("--markdown-output", required=True, type=Path)

    arguments = parser.parse_args()
    if arguments.command == "profile":
        segments = profile_segments(arguments.name)
        print(
            json.dumps(
                {
                    "profile": arguments.name,
                    "segments": list(segments),
                    "segment_count": len(segments),
                    "duration_seconds": sum(segments),
                },
                sort_keys=True,
            )
        )
        return 0
    if arguments.command == "run-segment":
        return run_segment(arguments)
    return aggregate(arguments)


if __name__ == "__main__":
    raise SystemExit(main())
