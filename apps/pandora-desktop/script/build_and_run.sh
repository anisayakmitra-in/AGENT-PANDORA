#!/usr/bin/env bash
set -euo pipefail

script_directory="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
desktop_root="$(cd "$script_directory/.." && pwd)"
debug=0
logs=0
telemetry=0
verify=0

usage() {
  echo "Usage: script/build_and_run.sh [--debug] [--logs] [--telemetry] [--verify]"
}

while (($#)); do
  case "$1" in
    --debug) debug=1 ;;
    --logs) logs=1 ;;
    --telemetry) telemetry=1 ;;
    --verify) verify=1 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; exit 2 ;;
  esac
  shift
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This entrypoint builds and launches the macOS app bundle." >&2
  exit 1
fi

cd "$desktop_root"
if ((verify)); then
  npm test
  npm run build
  cargo test --manifest-path src-tauri/Cargo.toml --locked
fi

pkill -x pandora-desktop 2>/dev/null || true
node script/stage-sidecar.mjs
npx tauri build --debug

app_bundle="$desktop_root/src-tauri/target/debug/bundle/macos/Pandora.app"
app_binary="$app_bundle/Contents/MacOS/pandora-desktop"
[[ -d "$app_bundle" && -x "$app_binary" ]] || { echo "Pandora.app was not produced" >&2; exit 1; }

if ((debug)); then
  exec lldb -- "$app_binary"
fi

open -n "$app_bundle"
if ((telemetry)); then
  telemetry_root="${PANDORA_DATA_DIR:-${HOME}/Library/Application Support/Pandora}/operations"
  echo "Local operational evidence: $telemetry_root"
  if [[ -d "$telemetry_root" ]]; then
    find "$telemetry_root" -maxdepth 2 -type f -print
  fi
fi
if ((logs)); then
  exec /usr/bin/log stream --style compact --predicate 'process == "pandora-desktop"'
fi
