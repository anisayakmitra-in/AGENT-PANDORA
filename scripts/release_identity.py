from __future__ import annotations

import argparse
import json
import re
import tomllib
from pathlib import Path


DESKTOP_WINDOWS_UPGRADE_CODE = "43f9019a-cb48-59a1-b463-5508bd89d386"


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
        with (root / "apps" / "pandora-desktop" / "package.json").open(
            encoding="utf-8"
        ) as package_file:
            desktop_npm_version = json.load(package_file)["version"]
        with (root / "apps" / "pandora-desktop" / "package-lock.json").open(
            encoding="utf-8"
        ) as package_lock_file:
            desktop_lock = json.load(package_lock_file)
        with (root / "apps" / "pandora-desktop" / "src-tauri" / "Cargo.toml").open(
            "rb"
        ) as desktop_cargo_file:
            desktop_cargo_version = tomllib.load(desktop_cargo_file)["package"][
                "version"
            ]
        with (
            root
            / "apps"
            / "pandora-desktop"
            / "src-tauri"
            / "tauri.conf.json"
        ).open(encoding="utf-8") as tauri_config_file:
            tauri_config = json.load(tauri_config_file)
            tauri_version_source = tauri_config["version"]
            windows_upgrade_code = tauri_config["bundle"]["windows"]["wix"][
                "upgradeCode"
            ]
        if not isinstance(version, str) or not version:
            raise ValueError("workspace package version is invalid")
        if npm_version != version:
            raise ValueError(
                f"npm package version {npm_version!r} does not match workspace {version!r}"
            )
        if desktop_npm_version != version:
            raise ValueError(
                "desktop npm package version "
                f"{desktop_npm_version!r} does not match workspace {version!r}"
            )
        desktop_lock_version = desktop_lock.get("version")
        desktop_lock_package_version = desktop_lock.get("packages", {}).get(
            "", {}
        ).get("version")
        if desktop_lock_version != version or desktop_lock_package_version != version:
            raise ValueError(
                "desktop package lock versions do not match workspace "
                f"{version!r}"
            )
        if desktop_cargo_version != version:
            raise ValueError(
                "desktop Cargo package version "
                f"{desktop_cargo_version!r} does not match workspace {version!r}"
            )
        if tauri_version_source != "../package.json":
            raise ValueError(
                "Tauri version must resolve from '../package.json' so desktop "
                "bundle metadata cannot drift"
            )
        if windows_upgrade_code != DESKTOP_WINDOWS_UPGRADE_CODE:
            raise ValueError(
                "desktop Windows MSI upgrade code changed; existing installs "
                "would no longer share one update identity"
            )
        expected_tag = f"v{version}"
        shell_installer = (root / "scripts" / "install.sh").read_text(encoding="utf-8")
        shell_defaults = re.findall(
            r'^version="\$\{PANDORA_VERSION:-(v[^"}]+)\}"$',
            shell_installer,
            re.MULTILINE,
        )
        if shell_defaults != [expected_tag]:
            raise ValueError(f"shell installer does not default to {expected_tag!r}")
        powershell_installer = (root / "scripts" / "install.ps1").read_text(
            encoding="utf-8"
        )
        powershell_defaults = re.findall(
            r'^\$defaultVersion = "(v[^"]+)"$', powershell_installer, re.MULTILINE
        )
        if powershell_defaults != [expected_tag]:
            raise ValueError(f"PowerShell installer does not default to {expected_tag!r}")
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
