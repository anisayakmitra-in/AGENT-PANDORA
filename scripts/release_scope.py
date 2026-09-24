from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path
from typing import Any


_RELEASE_TAG = re.compile(
    r"^v[0-9]+\.[0-9]+\.[0-9]+(?:-(alpha|beta|rc)\.[0-9]+)?$"
)
_ALLOWED_KEYS = {"schema_version", "scope", "desktop_required"}
_SCOPES = {"full", "cli-only"}
_MAX_POLICY_BYTES = 4096


class ReleaseScopeError(ValueError):
    pass


def channel_for_tag(tag: str) -> str:
    match = _RELEASE_TAG.fullmatch(tag)
    if match is None:
        raise ReleaseScopeError(f"invalid release tag: {tag}")
    prerelease = match.group(1)
    if prerelease is None:
        return "stable"
    if prerelease == "rc":
        return "release-candidate"
    return prerelease


def _read_policy(root: Path) -> dict[str, Any]:
    policy_path = root / "release-scope.json"
    try:
        metadata = policy_path.lstat()
    except FileNotFoundError as error:
        raise ReleaseScopeError("release-scope.json is missing") from error
    if metadata.st_size > _MAX_POLICY_BYTES:
        raise ReleaseScopeError("release-scope.json exceeds the size limit")
    if policy_path.is_symlink() or not policy_path.is_file():
        raise ReleaseScopeError("release-scope.json must be a regular file")
    try:
        document = json.loads(policy_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ReleaseScopeError("release-scope.json is invalid JSON") from error
    if not isinstance(document, dict) or set(document) != _ALLOWED_KEYS:
        raise ReleaseScopeError("release-scope.json has an unsupported shape")
    if type(document["schema_version"]) is not int or document["schema_version"] != 1:
        raise ReleaseScopeError("release-scope.json schema_version must be 1")
    scope = document["scope"]
    if type(scope) is not str:
        raise ReleaseScopeError("release-scope.json scope must be a string")
    if scope not in _SCOPES:
        raise ReleaseScopeError("release-scope.json scope is unsupported")
    desktop_required = document["desktop_required"]
    if type(desktop_required) is not bool:
        raise ReleaseScopeError("release-scope.json desktop_required must be boolean")
    if desktop_required != (scope == "full"):
        raise ReleaseScopeError("release scope and desktop_required disagree")
    return document


def resolve_release_scope(tag: str, root: Path) -> dict[str, object]:
    channel = channel_for_tag(tag)
    document = _read_policy(root.resolve(strict=True))
    scope = document["scope"]
    desktop_required = document["desktop_required"]
    if scope == "cli-only" and channel in {"release-candidate", "stable"}:
        raise ReleaseScopeError(
            f"{channel} release requires full scope, not cli-only"
        )
    return {
        "release_tag": tag,
        "channel": channel,
        "scope": scope,
        "desktop_required": desktop_required,
    }


def _write_github_output(path: Path, resolved: dict[str, object]) -> None:
    if path.is_symlink():
        raise ReleaseScopeError("GitHub output path must not be a symlink")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(
        f"scope={resolved['scope']}\n"
        f"channel={resolved['channel']}\n"
        f"desktop_required={str(resolved['desktop_required']).lower()}\n",
        encoding="utf-8",
    )


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate Pandora release scope")
    parser.add_argument("tag")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    parser.add_argument("--github-output", type=Path)
    arguments = parser.parse_args()

    try:
        resolved = resolve_release_scope(arguments.tag, arguments.root)
        if arguments.github_output is not None:
            _write_github_output(arguments.github_output, resolved)
    except (OSError, ReleaseScopeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    print(
        f"release scope {resolved['scope']} verified for "
        f"{resolved['channel']} tag {resolved['release_tag']}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
