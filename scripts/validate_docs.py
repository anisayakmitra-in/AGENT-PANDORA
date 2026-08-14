from __future__ import annotations

import argparse
import re
import subprocess
from pathlib import Path


UNFINISHED_MARKER = re.compile(r"\b(?:TODO|TBD|FIXME)\b", re.IGNORECASE)


def validate_text(relative: Path, text: str) -> list[str]:
    if UNFINISHED_MARKER.search(text):
        return [f"{relative.as_posix()}: contains unfinished documentation marker"]
    return []


def tracked_markdown(root: Path) -> list[Path]:
    result = subprocess.run(
        ["git", "-C", str(root), "ls-files", "-z", "--", "*.md"],
        check=True,
        capture_output=True,
    )
    return [Path(path) for path in result.stdout.decode("utf-8").split("\0") if path]


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate tracked Pandora documentation")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    arguments = parser.parse_args()
    root = arguments.root.resolve()
    findings: list[str] = []

    for relative in tracked_markdown(root):
        findings.extend(validate_text(relative, (root / relative).read_text(encoding="utf-8")))

    if findings:
        for finding in findings:
            print(f"error: {finding}")
        return 1

    print("documentation validation passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
