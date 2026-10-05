"""Render Homebrew, Scoop, and winget manifests from the release checksums.

Pandora publishes unsigned-by-OS binaries. Every package manager below installs
by checksum, so the checksum is the whole integrity story for each channel and
must come from the release's own checksum manifest rather than from a value
typed by hand.

This script only renders templates into release assets. It does not publish
anything. Publishing a Homebrew tap, a Scoop bucket, or a winget-pkgs pull
request each need an external repository that this project does not own; see
docs/PLATFORMS.md for those steps.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

try:
    from .installer_contract import expected_checksum, parse_checksums
except ImportError:
    if __package__ in (None, ""):  # pragma: no cover - direct script execution
        sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
    from scripts.installer_contract import expected_checksum, parse_checksums


REPOSITORY_ROOT = Path(__file__).resolve().parent.parent
TEMPLATE_ROOT = REPOSITORY_ROOT / "packaging"

# The binary each channel installs, and the asset that carries it.
CHANNELS: dict[str, dict[str, str]] = {
    "homebrew": {
        "template": "homebrew/pandora.rb.template",
        "output": "pandora.rb",
        "artifact": "pandora-aarch64-apple-darwin",
        "arch": "aarch64-apple-darwin",
    },
    "scoop": {
        "template": "scoop/pandora.json.template",
        "output": "pandora.json",
        "artifact": "pandora-x86_64-pc-windows-msvc.exe",
        "arch": "x64",
    },
    "winget": {
        "template": "winget/AnisayakmitraIn.AGENT-PANDORA.yaml.template",
        "output": "AnisayakmitraIn.AGENT-PANDORA.yaml",
        "artifact": "pandora-x86_64-pc-windows-msvc.exe",
        "arch": "x64",
    },
}

_PLACEHOLDER = re.compile(r"\{\{ ([A-Z0-9_]+) \}\}")
_RELEASE_TAG = re.compile(r"^v[0-9]+\.[0-9]+\.[0-9]+(?:-(?:alpha|beta|rc)\.[0-9]+)?$")


class ChannelManifestError(ValueError):
    pass


def _version_from_tag(tag: str) -> str:
    if _RELEASE_TAG.fullmatch(tag) is None:
        raise ChannelManifestError(f"invalid release tag: {tag}")
    return tag[1:]


def render_template(template: str, values: dict[str, str]) -> str:
    """Substitute {{ UPPER_SNAKE }} placeholders, rejecting unknown names.

    An unknown placeholder is an error rather than a silent passthrough: a
    manifest that still contains `{{ VERSION }}` would be published with a
    literal placeholder in it.
    """

    def substitute(match: re.Match[str]) -> str:
        name = match.group(1)
        if name not in values:
            raise ChannelManifestError(
                f"template placeholder has no value: {name}"
            )
        return values[name]

    rendered = _PLACEHOLDER.sub(substitute, template)
    leftover = _PLACEHOLDER.search(rendered)
    if leftover is not None:
        raise ChannelManifestError(
            f"template placeholder survived rendering: {leftover.group(1)}"
        )
    return rendered


def render_channel_manifests(
    manifest_path: Path,
    output_dir: Path,
    *,
    tag: str,
    template_root: Path | None = None,
    channels: dict[str, dict[str, str]] | None = None,
) -> dict[str, str]:
    """Render every configured channel. Returns the output paths written."""
    version = _version_from_tag(tag)
    root = template_root if template_root is not None else TEMPLATE_ROOT
    selected = channels if channels is not None else CHANNELS
    if not selected:
        raise ChannelManifestError("no channels configured to render")

    try:
        checksums = parse_checksums(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, ValueError) as error:
        raise ChannelManifestError(f"invalid checksum manifest: {error}") from error

    rendered: dict[str, str] = {}
    for channel, spec in sorted(selected.items()):
        artifact = spec["artifact"]
        try:
            digest = expected_checksum(checksums, artifact)
        except ValueError as error:
            raise ChannelManifestError(
                f"checksum manifest has no entry for {channel} artifact "
                f"{artifact}: {error}"
            ) from error

        template_path = root / spec["template"]
        try:
            template = template_path.read_text(encoding="utf-8")
        except OSError as error:
            raise ChannelManifestError(
                f"cannot read {channel} template {template_path}: {error}"
            ) from error

        content = render_template(
            template,
            {
                "VERSION": version,
                # The full git tag, including the leading "v". Package manager
                # URLs are built from the tag; the version field is not.
                "TAG": tag,
                "SHA256": digest,
                "ARTIFACT": artifact,
                "ARCH": spec["arch"],
                "CHANNEL": channel,
            },
        )

        destination = output_dir / spec["output"]
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(content, encoding="utf-8")
        rendered[channel] = str(destination)

    return rendered


def main() -> int:
    parser = argparse.ArgumentParser(
        description=(
            "Render Homebrew, Scoop, and winget manifests from the release "
            "checksum manifest. Renders only; it does not publish."
        )
    )
    parser.add_argument("tag")
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    arguments = parser.parse_args()

    try:
        rendered = render_channel_manifests(
            arguments.manifest,
            arguments.output_dir,
            tag=arguments.tag,
        )
    except (OSError, ChannelManifestError) as error:
        print(f"error: {error}")
        return 1

    for channel, path in sorted(rendered.items()):
        print(f"{channel}: {path}")
    print(
        "rendered; these are release assets only. Publishing to a Homebrew tap, "
        "Scoop bucket, or winget-pkgs requires an external repository."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())