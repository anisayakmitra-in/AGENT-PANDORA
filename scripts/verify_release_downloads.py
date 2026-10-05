from __future__ import annotations

import argparse
import hashlib
import shutil
import subprocess
from pathlib import Path

try:
    from .installer_contract import expected_checksum, parse_checksums
except ImportError:
    from installer_contract import expected_checksum, parse_checksums


class ReleaseDownloadError(ValueError):
    pass


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _verify_manifest_signature(
    manifest_path: Path,
    signature_path: Path,
    certificate_path: Path,
    *,
    identity: str,
    issuer: str,
) -> None:
    """Authenticate the manifest with cosign before trusting any checksum in it.

    A checksum only proves an artifact matches a manifest. If the manifest
    itself is unauthenticated, anything able to serve both files controls the
    result. So this is fail-closed: if a signature is offered it must verify,
    and an unavailable or failing cosign is an error rather than a skip.
    """
    for path, label in (
        (signature_path, "signature"),
        (certificate_path, "certificate"),
    ):
        if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
            raise ReleaseDownloadError(
                f"release manifest {label} is missing or empty: {path.name}"
            )

    cosign = shutil.which("cosign")
    if cosign is None:
        raise ReleaseDownloadError(
            "cosign is required to verify the checksum manifest signature; "
            "install it from https://github.com/sigstore/cosign/releases"
        )

    completed = subprocess.run(
        [
            cosign,
            "verify-blob",
            str(manifest_path),
            "--certificate",
            str(certificate_path),
            "--signature",
            str(signature_path),
            "--certificate-identity",
            identity,
            "--certificate-oidc-issuer",
            issuer,
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    if completed.returncode != 0:
        detail = (completed.stderr or completed.stdout or "").strip()
        raise ReleaseDownloadError(
            f"release manifest signature verification failed: {detail}"
        )


def verify_release_evidence_signature(
    manifest_path: Path,
    *,
    signature_path: Path | None = None,
    certificate_path: Path | None = None,
    identity: str | None = None,
    issuer: str = "https://token.actions.githubusercontent.com",
    required: bool = False,
) -> None:
    """Verify the manifest signature when it is present, or when required.

    A signature that exists but does not verify is always an error. This is the
    rule the installers follow too: a present signature is a claim, and an
    unverifiable claim is a failure.
    """
    resolved_signature = (
        signature_path if signature_path is not None else manifest_path.with_suffix(".txt.sig")
    )
    resolved_certificate = (
        certificate_path
        if certificate_path is not None
        else manifest_path.with_suffix(".txt.pem")
    )

    present = resolved_signature.is_file() or resolved_certificate.is_file()
    if not present:
        if required:
            raise ReleaseDownloadError(
                "release manifest signature is required but not present: "
                f"{resolved_signature.name}, {resolved_certificate.name}"
            )
        return

    if identity is None:
        raise ReleaseDownloadError(
            "a certificate identity is required to verify the manifest signature"
        )
    _verify_manifest_signature(
        manifest_path,
        resolved_signature,
        resolved_certificate,
        identity=identity,
        issuer=issuer,
    )


def verify_release_downloads(
    manifest_path: Path,
    artifacts: list[Path],
    *,
    identity: str | None = None,
    issuer: str = "https://token.actions.githubusercontent.com",
    require_signature: bool = False,
) -> None:
    if manifest_path.is_symlink() or not manifest_path.is_file():
        raise ReleaseDownloadError("checksum manifest must be a regular file")
    if not artifacts:
        raise ReleaseDownloadError("at least one release artifact is required")

    # Authenticate the manifest before reading a single checksum out of it.
    verify_release_evidence_signature(
        manifest_path,
        identity=identity,
        issuer=issuer,
        required=require_signature,
    )

    try:
        manifest = parse_checksums(manifest_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, ValueError) as error:
        raise ReleaseDownloadError(f"invalid checksum manifest: {error}") from error

    seen: set[str] = set()
    for artifact in artifacts:
        if artifact.is_symlink() or not artifact.is_file():
            raise ReleaseDownloadError(
                f"release artifact must be a regular file: {artifact}"
            )
        name = artifact.name
        if name in seen:
            raise ReleaseDownloadError(f"duplicate release artifact: {name}")
        seen.add(name)
        try:
            expected = expected_checksum(manifest, name)
        except ValueError as error:
            raise ReleaseDownloadError(str(error)) from error
        actual = sha256_file(artifact)
        if actual != expected:
            raise ReleaseDownloadError(
                f"checksum mismatch for {name}: expected {expected}, got {actual}"
            )


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Verify downloaded Pandora release artifacts against checksums.txt"
    )
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument(
        "--certificate-identity",
        help=(
            "cosign certificate identity. Required when the release manifest has a "
            "signature alongside it."
        ),
    )
    parser.add_argument(
        "--certificate-oidc-issuer",
        default="https://token.actions.githubusercontent.com",
    )
    parser.add_argument(
        "--require-signature",
        action="store_true",
        help="fail if the manifest signature is absent, not only if it is invalid",
    )
    parser.add_argument("artifacts", nargs="+", type=Path)
    arguments = parser.parse_args()

    try:
        verify_release_downloads(
            arguments.manifest,
            arguments.artifacts,
            identity=arguments.certificate_identity,
            issuer=arguments.certificate_oidc_issuer,
            require_signature=arguments.require_signature,
        )
    except (OSError, ReleaseDownloadError) as error:
        print(f"error: {error}")
        return 1

    print(f"verified {len(arguments.artifacts)} downloaded release artifacts")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
