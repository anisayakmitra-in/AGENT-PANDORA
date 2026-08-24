import argparse
from pathlib import Path
import re
import sys


_RELEASE_HEADING = re.compile(r"^##\s+(v\S+)\s*$")
_RELEASE_TAG = re.compile(
    r"^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-(alpha|beta|rc)\.(0|[1-9][0-9]*))?$"
)
_STAGE_ORDER = {"alpha": 0, "beta": 1, "rc": 2, None: 3}


class ReleasePredecessorError(ValueError):
    pass


def _parse_tag(tag: str) -> tuple[int, int, int, int, int]:
    match = _RELEASE_TAG.fullmatch(tag)
    if match is None:
        raise ReleasePredecessorError(f"invalid release tag: {tag}")
    stage = match.group(4)
    return (
        int(match.group(1)),
        int(match.group(2)),
        int(match.group(3)),
        _STAGE_ORDER[stage],
        int(match.group(5) or 0),
    )


def find_predecessor(changelog: str, current_tag: str) -> str:
    current_version = _parse_tag(current_tag)
    releases: list[tuple[str, tuple[int, int, int, int, int]]] = []
    seen: set[str] = set()
    for line in changelog.splitlines():
        match = _RELEASE_HEADING.fullmatch(line)
        if match is None:
            continue
        tag = match.group(1)
        version = _parse_tag(tag)
        if tag in seen:
            raise ReleasePredecessorError(f"duplicate release section: {tag}")
        seen.add(tag)
        releases.append((tag, version))

    current_indexes = [index for index, (tag, _) in enumerate(releases) if tag == current_tag]
    if not current_indexes:
        raise ReleasePredecessorError(
            f"current release section is missing: {current_tag}"
        )
    current_index = current_indexes[0]
    if current_index + 1 >= len(releases):
        raise ReleasePredecessorError(
            f"predecessor release section is missing: {current_tag}"
        )
    predecessor_tag, predecessor_version = releases[current_index + 1]
    if predecessor_version >= current_version:
        raise ReleasePredecessorError(
            f"predecessor {predecessor_tag} must be older than {current_tag}"
        )
    return predecessor_tag


def main(arguments: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description="Select the previous Pandora release from the changelog"
    )
    parser.add_argument("current_tag")
    parser.add_argument(
        "--changelog",
        type=Path,
        default=Path(__file__).resolve().parents[1] / "CHANGELOG.md",
    )
    options = parser.parse_args(arguments)
    try:
        changelog = options.changelog.read_text(encoding="utf-8")
        predecessor = find_predecessor(changelog, options.current_tag)
    except (OSError, ReleasePredecessorError) as error:
        print(f"release predecessor: {error}", file=sys.stderr)
        return 1
    print(predecessor)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
