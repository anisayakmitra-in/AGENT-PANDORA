from __future__ import annotations

import argparse
import json
import tomllib
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate Pandora release identity")
    parser.add_argument("tag", nargs="?")
    parser.add_argument("--root", type=Path, default=Path.cwd())
    arguments = parser.parse_args()

    try:
        root = arguments.root.resolve()
        with (root / "Cargo.toml").open("rb") as cargo_file:
            version = tomllib.load(cargo_file)["workspace"]["package"]["version"]
        with (root / "npm" / "pandora-cli" / "package.json").open(
            encoding="utf-8"
        ) as package_file:
            npm_version = json.load(package_file)["version"]
        if not isinstance(version, str) or not version:
            raise ValueError("workspace package version is invalid")
        if npm_version != version:
            raise ValueError(
                f"npm package version {npm_version!r} does not match workspace {version!r}"
            )
        expected_tag = f"v{version}"
        if arguments.tag is not None and arguments.tag != expected_tag:
            raise ValueError(
                f"release tag {arguments.tag!r} does not match {expected_tag!r}"
            )
    except (KeyError, OSError, ValueError) as error:
        print(f"error: {error}")
        return 1

    print(f"release identity {expected_tag} verified")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
