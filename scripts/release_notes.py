from __future__ import annotations

import argparse
import sys
from pathlib import Path


def extract_release_notes(changelog: str, tag: str) -> str:
    heading = f"## {tag}"
    lines = changelog.splitlines()
    try:
        start = lines.index(heading) + 1
    except ValueError as error:
        raise ValueError(f"release notes for {tag} are missing from CHANGELOG.md") from error

    end = next(
        (index for index in range(start, len(lines)) if lines[index].startswith("## ")),
        len(lines),
    )
    notes = "\n".join(lines[start:end]).strip()
    if not notes:
        raise ValueError(f"release notes for {tag} are empty")
    return notes


def main() -> int:
    parser = argparse.ArgumentParser(description="Extract one tagged Pandora release section")
    parser.add_argument("tag", help="release tag, for example v2.0.0-beta.1")
    parser.add_argument("--changelog", type=Path, default=Path("CHANGELOG.md"))
    arguments = parser.parse_args()

    try:
        notes = extract_release_notes(
            arguments.changelog.read_text(encoding="utf-8"), arguments.tag
        )
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    print(notes)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
