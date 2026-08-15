from __future__ import annotations

import argparse
import re
import subprocess
from pathlib import Path
from urllib.parse import unquote


UNFINISHED_MARKER = re.compile(r"\b(?:TODO|TBD|FIXME)\b", re.IGNORECASE)
LOCAL_LINK = re.compile(r"(?<!!)\[[^\]]+\]\(([^)\s]+)")


def validate_text(relative: Path, text: str, root: Path | None = None) -> list[str]:
    findings: list[str] = []
    if UNFINISHED_MARKER.search(text):
        findings.append(f"{relative.as_posix()}: contains unfinished documentation marker")

    repository_root = (root or Path.cwd()).resolve()
    for raw_target in LOCAL_LINK.findall(text):
        target = raw_target.strip().strip("<>")
        if not target or target.startswith(("#", "/")) or re.match(
            r"^[A-Za-z][A-Za-z0-9+.-]*:", target
        ):
            continue
        target = unquote(target.split("#", 1)[0])
        if not target:
            continue
        resolved = (repository_root / relative.parent / target).resolve()
        try:
            resolved.relative_to(repository_root)
        except ValueError:
            findings.append(f"{relative.as_posix()}: local link escapes repository: {raw_target}")
            continue
        if not resolved.exists():
            findings.append(f"{relative.as_posix()}: local link does not exist: {raw_target}")
    return findings


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
        findings.extend(
            validate_text(relative, (root / relative).read_text(encoding="utf-8"), root)
        )

    if findings:
        for finding in findings:
            print(f"error: {finding}")
        return 1

    print("documentation validation passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
