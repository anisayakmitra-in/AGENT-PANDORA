#!/bin/sh
set -eu

fail() {
  printf '%s\n' "pandora installer: $1" >&2
  exit 1
}

version="${PANDORA_VERSION:-v2.0.0-alpha.6}"
printf '%s\n' "$version" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$' \
  || fail "PANDORA_VERSION must be a SemVer tag such as v2.0.0-alpha.6"
case "$version" in
  *[!A-Za-z0-9._-]*) fail "PANDORA_VERSION contains unsafe characters" ;;
esac

base="${PANDORA_RELEASE_BASE_URL:-https://github.com/anisayakmitra-in/PANDORA-AGENT/releases/download}"
case "$base" in
  https://*) ;;
  *) fail "PANDORA_RELEASE_BASE_URL must use HTTPS" ;;
esac
case "$base" in
  *\?*|*#*|*@*) fail "PANDORA_RELEASE_BASE_URL must not contain credentials or query parameters" ;;
esac

platform="$(uname -s)"
architecture="$(uname -m)"
case "$platform:$architecture" in
  Linux:x86_64|Linux:amd64) artifact="pandora-x86_64-unknown-linux-gnu" ;;
  Darwin:x86_64|Darwin:amd64) artifact="pandora-x86_64-apple-darwin" ;;
  Darwin:arm64|Darwin:aarch64) artifact="pandora-aarch64-apple-darwin" ;;
  *) fail "unsupported platform or architecture: $platform $architecture" ;;
esac

install_dir="${PANDORA_INSTALL_DIR:-$HOME/.local/bin}"
temporary="$(mktemp -d "${TMPDIR:-/tmp}/pandora-install.XXXXXX")"
trap 'rm -rf "$temporary"' EXIT INT TERM

download() {
  curl --fail --silent --show-error --location --retry 3 --proto '=https' --tlsv1.2 \
    --output "$2" "$1"
}

base="${base%/}"
download "$base/$version/checksums.txt" "$temporary/checksums.txt"
download "$base/$version/$artifact" "$temporary/$artifact"

expected="$(awk -v name="$artifact" '$2 == name || $2 == "*" name { print $1; exit }' "$temporary/checksums.txt")"
printf '%s' "$expected" | grep -Eq '^[0-9A-Fa-f]{64}$' || fail "release checksum is missing or malformed"
if command -v sha256sum >/dev/null 2>&1; then
  actual="$(sha256sum "$temporary/$artifact" | awk '{print $1}')"
else
  actual="$(shasum -a 256 "$temporary/$artifact" | awk '{print $1}')"
fi
[ "$(printf '%s' "$actual" | tr '[:upper:]' '[:lower:]')" = "$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')" ] \
  || fail "release checksum verification failed"

if [ "${PANDORA_REQUIRE_SIGNATURE:-0}" = "1" ]; then
  command -v cosign >/dev/null 2>&1 || fail "cosign is required when PANDORA_REQUIRE_SIGNATURE=1"
  identity="${PANDORA_COSIGN_IDENTITY:-}"
  [ -n "$identity" ] || fail "PANDORA_COSIGN_IDENTITY is required for signature verification"
  download "$base/$version/checksums.txt.sig" "$temporary/checksums.txt.sig"
  download "$base/$version/checksums.txt.pem" "$temporary/checksums.txt.pem"
  cosign verify-blob "$temporary/checksums.txt" \
    --certificate "$temporary/checksums.txt.pem" \
    --signature "$temporary/checksums.txt.sig" \
    --certificate-identity "$identity" \
    --certificate-oidc-issuer "${PANDORA_COSIGN_OIDC_ISSUER:-https://token.actions.githubusercontent.com}" \
    >/dev/null || fail "release signature verification failed"
fi

mkdir -p "$install_dir"
target="$install_dir/pandora"
[ ! -L "$target" ] || fail "refusing to replace symlink: $target"
staged="$install_dir/.pandora.$$.new"
cp "$temporary/$artifact" "$staged"
chmod 0755 "$staged"
mv -f "$staged" "$target"
printf 'Pandora installed at %s\n' "$target"
