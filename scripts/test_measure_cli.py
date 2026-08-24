import json
import math
import os
import stat
import subprocess
import sys
import unittest
import uuid
from pathlib import Path

from scripts.measure_cli import MeasurementError, summarize_elapsed


ROOT = Path(__file__).resolve().parent.parent
MEASURE = ROOT / "scripts" / "measure_cli.py"


class FakeCli:
    def __init__(self) -> None:
        self.root = ROOT / "scripts" / f"measure-test-{uuid.uuid4().hex}"
        self.root.mkdir()
        self.log = self.root / "calls.jsonl"
        program = self.root / "fake_cli.py"
        program.write_text(
            """import json
import os
import sys
import time

arguments = sys.argv[1:]
command = "version" if arguments[:2] == ["--version", "--json"] else arguments[0]
with open(os.environ["FAKE_CLI_LOG"], "a", encoding="utf-8") as log:
    log.write(json.dumps(arguments) + "\\n")
if os.environ.get("FAKE_CLI_TIMEOUT") == command:
    time.sleep(5)
if os.environ.get("FAKE_CLI_FAILURE") == command:
    raise SystemExit(9)
print(json.dumps({"version": "0.1", "command": command}))
""",
            encoding="utf-8",
        )
        if os.name == "nt":
            self.binary = self.root / "fake-pandora.cmd"
            self.binary.write_text(
                f'@echo off\r\n"{sys.executable}" "{program}" %*\r\n',
                encoding="utf-8",
            )
        else:
            self.binary = self.root / "fake-pandora"
            self.binary.write_text(
                f'#!/bin/sh\nexec "{sys.executable}" "{program}" "$@"\n',
                encoding="utf-8",
            )
            self.binary.chmod(self.binary.stat().st_mode | stat.S_IXUSR)

    def environment(self) -> dict[str, str]:
        environment = os.environ.copy()
        environment["FAKE_CLI_LOG"] = str(self.log)
        return environment

    def cleanup(self) -> None:
        for path in sorted(self.root.rglob("*"), reverse=True):
            if path.is_file():
                path.unlink()
            else:
                path.rmdir()
        self.root.rmdir()


class MeasureCliTests(unittest.TestCase):
    def test_ci_records_cross_platform_baseline_artifacts(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yml").read_text(
            encoding="utf-8"
        )

        build_step = workflow.index("- name: Build release CLI")
        measure_step = workflow.index("- name: Measure CLI baseline")
        upload_step = workflow.index("- name: Upload CLI baseline")

        self.assertLess(build_step, measure_step)
        self.assertLess(measure_step, upload_step)
        self.assertIn("--binary target/release/pandora ", workflow)
        self.assertIn("--binary target/release/pandora.exe ", workflow)
        self.assertIn("--iterations 5 --timeout-seconds 10", workflow)
        self.assertIn(
            "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02",
            workflow,
        )
        self.assertIn("name: cli-baseline-${{ runner.os }}-${{ runner.arch }}", workflow)
        self.assertIn("if-no-files-found: error", workflow)

    def test_records_bounded_version_and_doctor_samples(self) -> None:
        fake = FakeCli()
        output = fake.root / "baseline.json"
        try:
            result = subprocess.run(
                [
                    sys.executable,
                    str(MEASURE),
                    "--binary",
                    str(fake.binary),
                    "--iterations",
                    "3",
                    "--timeout-seconds",
                    "2",
                    "--output",
                    str(output),
                ],
                capture_output=True,
                text=True,
                env=fake.environment(),
                check=False,
            )

            self.assertEqual(result.returncode, 0, result.stderr)
            report = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(json.loads(result.stdout), report)
            self.assertEqual(report["version"], "0.1")
            self.assertEqual(report["binary"], fake.binary.name)
            self.assertEqual(report["iterations"], 3)
            self.assertEqual(report["timeout_seconds"], 2)
            self.assertIn("os", report["platform"])
            self.assertIn("architecture", report["platform"])
            for command in ("version", "doctor"):
                measurement = report["commands"][command]
                self.assertEqual(measurement["attempts"], 3)
                self.assertEqual(measurement["successes"], 3)
                self.assertEqual(measurement["failures"], 0)
                self.assertEqual(measurement["timeouts"], 0)
                self.assertEqual(len(measurement["samples"]), 3)
                self.assertGreaterEqual(measurement["median_ms"], 0)
                self.assertGreaterEqual(measurement["p95_ms"], 0)
                for sample in measurement["samples"]:
                    self.assertTrue(sample["success"])
                    self.assertEqual(sample["exit_code"], 0)
                    self.assertFalse(sample["timed_out"])
                    self.assertGreaterEqual(sample["elapsed_ms"], 0)

            calls = [
                json.loads(line)
                for line in fake.log.read_text(encoding="utf-8").splitlines()
            ]
            self.assertEqual(calls.count(["setup", "--json"]), 1)
            self.assertEqual(calls.count(["--version", "--json"]), 3)
            self.assertEqual(calls.count(["doctor", "--json"]), 3)
        finally:
            fake.cleanup()

    def test_timeout_is_recorded_and_fails_the_measurement(self) -> None:
        fake = FakeCli()
        output = fake.root / "timeout.json"
        environment = fake.environment()
        environment["FAKE_CLI_TIMEOUT"] = "doctor"
        try:
            result = subprocess.run(
                [
                    sys.executable,
                    str(MEASURE),
                    "--binary",
                    str(fake.binary),
                    "--iterations",
                    "1",
                    "--timeout-seconds",
                    "1",
                    "--output",
                    str(output),
                ],
                capture_output=True,
                text=True,
                env=environment,
                check=False,
            )

            self.assertEqual(result.returncode, 1)
            report = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(report["commands"]["version"]["successes"], 1)
            doctor = report["commands"]["doctor"]
            self.assertEqual(doctor["successes"], 0)
            self.assertEqual(doctor["failures"], 1)
            self.assertEqual(doctor["timeouts"], 1)
            self.assertIsNone(doctor["samples"][0]["exit_code"])
            self.assertTrue(doctor["samples"][0]["timed_out"])
        finally:
            fake.cleanup()

    def test_command_failure_is_recorded_and_fails_the_measurement(self) -> None:
        fake = FakeCli()
        output = fake.root / "failure.json"
        environment = fake.environment()
        environment["FAKE_CLI_FAILURE"] = "doctor"
        try:
            result = subprocess.run(
                [
                    sys.executable,
                    str(MEASURE),
                    "--binary",
                    str(fake.binary),
                    "--iterations",
                    "1",
                    "--timeout-seconds",
                    "2",
                    "--output",
                    str(output),
                ],
                capture_output=True,
                text=True,
                env=environment,
                check=False,
            )

            self.assertEqual(result.returncode, 1)
            doctor = json.loads(output.read_text(encoding="utf-8"))["commands"][
                "doctor"
            ]
            self.assertEqual(doctor["successes"], 0)
            self.assertEqual(doctor["failures"], 1)
            self.assertEqual(doctor["timeouts"], 0)
            self.assertEqual(doctor["samples"][0]["exit_code"], 9)
            self.assertFalse(doctor["samples"][0]["timed_out"])
        finally:
            fake.cleanup()

    def test_rejects_unbounded_options_and_non_finite_samples(self) -> None:
        fake = FakeCli()
        try:
            for option, value in (
                ("--iterations", "0"),
                ("--iterations", "101"),
                ("--timeout-seconds", "0"),
                ("--timeout-seconds", "61"),
            ):
                result = subprocess.run(
                    [
                        sys.executable,
                        str(MEASURE),
                        "--binary",
                        str(fake.binary),
                        option,
                        value,
                        "--output",
                        str(fake.root / "invalid.json"),
                    ],
                    capture_output=True,
                    text=True,
                    check=False,
                )
                self.assertEqual(result.returncode, 2)
                self.assertIn("between", result.stderr)

            with self.assertRaises(MeasurementError):
                summarize_elapsed([math.nan])
        finally:
            fake.cleanup()


if __name__ == "__main__":
    unittest.main()
