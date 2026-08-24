from __future__ import annotations

import argparse
import json
import math
import os
import platform
import shutil
import statistics
import subprocess
import sys
import time
import uuid
from pathlib import Path
from typing import Any


REPORT_VERSION = "0.1"


class MeasurementError(ValueError):
    pass


def bounded_integer(name: str, minimum: int, maximum: int):
    def parse(value: str) -> int:
        try:
            parsed = int(value)
        except ValueError as error:
            raise argparse.ArgumentTypeError(f"{name} must be an integer") from error
        if not minimum <= parsed <= maximum:
            raise argparse.ArgumentTypeError(
                f"{name} must be between {minimum} and {maximum}"
            )
        return parsed

    return parse


def summarize_elapsed(values: list[float]) -> tuple[float, float]:
    if not values or any(not math.isfinite(value) or value < 0 for value in values):
        raise MeasurementError("elapsed samples must be finite non-negative values")
    ordered = sorted(values)
    p95_index = max(0, math.ceil(len(ordered) * 0.95) - 1)
    return round(statistics.median(ordered), 3), round(ordered[p95_index], 3)


def valid_json_response(payload: bytes, command: str) -> bool:
    try:
        value = json.loads(payload.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        return False
    return (
        isinstance(value, dict)
        and value.get("version") == REPORT_VERSION
        and value.get("command") == command
    )


def measure_command(
    binary: Path,
    arguments: list[str],
    command: str,
    iterations: int,
    timeout_seconds: int,
    environment: dict[str, str],
) -> dict[str, Any]:
    samples: list[dict[str, Any]] = []
    for iteration in range(1, iterations + 1):
        started = time.perf_counter_ns()
        try:
            completed = subprocess.run(
                [str(binary), *arguments],
                capture_output=True,
                env=environment,
                timeout=timeout_seconds,
                check=False,
            )
            timed_out = False
            exit_code = completed.returncode
            success = exit_code == 0 and valid_json_response(completed.stdout, command)
        except subprocess.TimeoutExpired:
            timed_out = True
            exit_code = None
            success = False
        elapsed_ms = round((time.perf_counter_ns() - started) / 1_000_000, 3)
        if not math.isfinite(elapsed_ms):
            raise MeasurementError("elapsed measurement is not finite")
        samples.append(
            {
                "iteration": iteration,
                "elapsed_ms": elapsed_ms,
                "success": success,
                "exit_code": exit_code,
                "timed_out": timed_out,
            }
        )

    elapsed = [sample["elapsed_ms"] for sample in samples]
    median_ms, p95_ms = summarize_elapsed(elapsed)
    successes = sum(1 for sample in samples if sample["success"])
    return {
        "attempts": iterations,
        "successes": successes,
        "failures": iterations - successes,
        "timeouts": sum(1 for sample in samples if sample["timed_out"]),
        "median_ms": median_ms,
        "p95_ms": p95_ms,
        "samples": samples,
    }


def prepare_environment(
    binary: Path, output: Path, timeout_seconds: int
) -> tuple[dict[str, str], Path]:
    root = output.parent / f".pandora-cli-baseline-{uuid.uuid4().hex}"
    root.mkdir()
    environment = os.environ.copy()
    environment["PANDORA_CONFIG"] = str(root / "config.json")
    environment["PANDORA_DATA_DIR"] = str(root / "data")
    environment["PANDORA_WORKSPACE"] = str(root / "workspace")
    environment.pop("PANDORA_PROVIDER_URL", None)
    try:
        setup = subprocess.run(
            [str(binary), "setup", "--json"],
            capture_output=True,
            env=environment,
            timeout=timeout_seconds,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        shutil.rmtree(root)
        raise MeasurementError("setup timed out") from error
    if setup.returncode != 0 or not valid_json_response(setup.stdout, "setup"):
        shutil.rmtree(root)
        raise MeasurementError("setup did not return a successful JSON response")
    return environment, root


def build_report(
    binary: Path, iterations: int, timeout_seconds: int, output: Path
) -> dict[str, Any]:
    environment, temporary_root = prepare_environment(binary, output, timeout_seconds)
    try:
        commands = {
            "version": measure_command(
                binary,
                ["--version", "--json"],
                "version",
                iterations,
                timeout_seconds,
                environment,
            ),
            "doctor": measure_command(
                binary,
                ["doctor", "--json"],
                "doctor",
                iterations,
                timeout_seconds,
                environment,
            ),
        }
    finally:
        shutil.rmtree(temporary_root)
    return {
        "version": REPORT_VERSION,
        "binary": binary.name,
        "iterations": iterations,
        "timeout_seconds": timeout_seconds,
        "platform": {
            "os": platform.system().lower(),
            "architecture": platform.machine().lower(),
        },
        "commands": commands,
    }


def main(arguments: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Record bounded Pandora CLI reliability and latency samples"
    )
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument(
        "--iterations",
        type=bounded_integer("iterations", 1, 100),
        default=10,
    )
    parser.add_argument(
        "--timeout-seconds",
        type=bounded_integer("timeout-seconds", 1, 60),
        default=10,
    )
    parser.add_argument("--output", type=Path, required=True)
    options = parser.parse_args(arguments)
    binary = options.binary.resolve()
    output = options.output.resolve()
    if not binary.is_file():
        parser.error("binary must be an existing file")
    if not output.parent.is_dir():
        parser.error("output parent must be an existing directory")

    try:
        report = build_report(binary, options.iterations, options.timeout_seconds, output)
        rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
        output.write_text(rendered, encoding="utf-8")
    except (MeasurementError, OSError) as error:
        print(f"measurement failed: {error}", file=sys.stderr)
        return 1

    print(json.dumps(report, sort_keys=True))
    return 1 if any(command["failures"] for command in report["commands"].values()) else 0


if __name__ == "__main__":
    raise SystemExit(main())
