#!/usr/bin/env bash
#
# Runs the CI verify job locally, in CI's order, and prints a summary.
#
# This mirrors the `verify` job in .github/workflows/ci.yml. The commands are
# transcribed from that file, not from memory; if you change ci.yml, change
# this script in the same commit or it stops being a gate.
#
# STEP 1 shipped a formatting failure that every local check passed, because the
# local check enumerated lib.rs, main.rs and tests/*.rs but not src/bin/*.rs. A
# gate that reimplements the job is a gate that can drift from it. This runs the
# job's commands, so the file list is never ours to get wrong.
#
# Steps that are environment provisioning in CI (checkout, toolchain install,
# cache restore, artifact upload) have no local equivalent and are reported as
# SKIPPED rather than silently dropped.
#
# Exit code 0 if every runnable step passed, 1 otherwise.
#
# Usage:
#   scripts/prepush.sh          # skips the slow release-path steps
#   scripts/prepush.sh --full   # also runs cargo install and the perf baseline

set -uo pipefail

FULL=0
for arg in "$@"; do
  case "$arg" in
    --full) FULL=1 ;;
    *) echo "unknown argument: $arg" >&2; exit 2 ;;
  esac
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT" || exit 1

# CI pins Python 3.11 and runs it as `python`. On Unix that is `python3`; some
# minimal images only expose `python`. Resolving once here keeps every later
# reference correct and means a missing interpreter fails loudly at the top
# rather than as a confusing per-step failure.
PYTHON=''
for candidate in python3 python; do
  if command -v "$candidate" >/dev/null 2>&1 &&
     "$candidate" -c 'import sys; raise SystemExit(0 if sys.version_info[:2] == (3, 11) else 1)' >/dev/null 2>&1; then
    PYTHON="$candidate"
    break
  fi
done
if [ -z "$PYTHON" ]; then
  echo "prepush: no Python 3.11 interpreter on PATH (CI pins python-version 3.11)" >&2
  echo "         tried: python3, python" >&2
  exit 1
fi

RESULTS=()

record() { # name status note
  RESULTS+=("$(printf '%s\t%s\t%s' "$1" "$2" "${3:-}")")
}

run_step() { # name working_dir command...
  local name="$1"; shift
  local workdir="$1"; shift
  local cmd="$*"

  printf '\n==> %s\n' "$name"
  printf '    %s\n' "$cmd"

  # The command arrives as a string so $PYTHON and $TMPDIR expand at call time,
  # inside the subshell that has already cd'd to the right directory.
  local output status
  output="$( cd "$workdir" && eval "$cmd" 2>&1 )"
  status=$?

  if [ "$status" -ne 0 ]; then
    printf '%s\n' "$output"
    record "$name" FAIL ''
    return 1
  fi

  printf '%s\n' "$output" | grep -v '^[[:space:]]*$' | tail -n 6
  record "$name" PASS ''
  return 0
}

echo "Verifying against the CI verify job in .github/workflows/ci.yml"
echo "  repo:     $REPO_ROOT"
echo "  cargo:    $(cargo --version)"
echo "  rustc:    $(rustc --version)"
echo "  python:   $($PYTHON --version 2>&1)"
echo "  node:     $(node --version 2>&1)"

# ---------------------------------------------------------------------------
# Setup steps. Provisioning in CI; here they are assertions that the local
# toolchain is the one CI pins, because a gate run on the wrong rustc proves
# nothing about the merge.
# ---------------------------------------------------------------------------
record 'Check out source' SKIP 'provisioning in CI'
record 'Install Rust' SKIP "local toolchain is pinned by rust-toolchain.toml: $(rustup show active-toolchain 2>/dev/null)"
record 'Restore Rust build cache' SKIP 'provisioning in CI'

NODE_VERSION="$(node --version 2>/dev/null)"
case "$NODE_VERSION" in
  v22.*) ;;
  *) echo "    WARNING: CI pins Node v22, local is $NODE_VERSION" ;;
esac

# ---------------------------------------------------------------------------
# The verify job, in order.
# ---------------------------------------------------------------------------
FAILED=0

run_step 'Check formatting' '.' 'cargo fmt --all -- --check' || FAILED=1
run_step 'Check workspace' '.' 'cargo check --workspace --locked' || FAILED=1
run_step 'Run Clippy' '.' 'cargo clippy --workspace --all-targets --locked -- -D warnings' || FAILED=1
run_step 'Run Rust tests' '.' 'cargo test --workspace --lib --tests --locked' || FAILED=1

run_step 'Build TypeScript launcher boundary' 'npm/pandora-cli' \
  'npm ci --ignore-scripts && npm run build' || FAILED=1

run_step 'Check TypeScript launcher output is current' '.' \
  'git diff --exit-code -- npm/pandora-cli/lib' || FAILED=1

run_step 'Test npm launcher' '.' 'node scripts/test_npm_launcher.js' || FAILED=1
run_step 'Test TypeScript client' '.' 'node scripts/test_typescript_client.js' || FAILED=1

run_step 'Run Python tests' '.' '$PYTHON -m unittest discover -s scripts -p "test_*.py" -v' || FAILED=1

# No GITHUB_TOKEN locally, so this step's own comment says it will skip rather
# than fail. Passing it through unchanged keeps the behaviour honest.
run_step 'Check workflow hardening' '.' \
  '$PYTHON -m unittest scripts.test_workflow_hardening -v' || FAILED=1

run_step 'Validate repository' '.' '$PYTHON scripts/validate_repo.py' || FAILED=1
run_step 'Validate documentation' '.' '$PYTHON scripts/validate_docs.py' || FAILED=1

run_step 'Build release CLI' '.' 'cargo build --release -p pandora-cli --locked' || FAILED=1

if [ "$FULL" -eq 1 ]; then
  INSTALL_ROOT="${TMPDIR:-/tmp}/pandora-cargo-install"
  run_step 'Verify cargo-installable CLI (Unix)' '.' \
    "cargo install --path crates/pandora-cli --locked --root '$INSTALL_ROOT' --force && '$INSTALL_ROOT/bin/pandora' --version" || FAILED=1

  run_step 'Measure CLI baseline' '.' \
    "$PYTHON scripts/measure_cli.py --binary target/release/pandora --iterations 5 --timeout-seconds 10 --output '${TMPDIR:-/tmp}/pandora-cli-baseline.json'" || FAILED=1
else
  record 'Verify cargo-installable CLI (Unix)' SKIP 'pass --full to run; it recompiles the CLI from scratch'
  record 'Measure CLI baseline' SKIP 'pass --full to run; timing-sensitive and takes minutes'
fi

record 'Upload CLI baseline' SKIP 'artifact upload, no local equivalent'

# ---------------------------------------------------------------------------
# Summary. Shaped to paste into a PR description.
# ---------------------------------------------------------------------------
WIDTH=0
for row in "${RESULTS[@]}"; do
  name="${row%%$'\t'*}"
  [ "${#name}" -gt "$WIDTH" ] && WIDTH="${#name}"
done

# A leading `-` would be parsed as a printf option, so the rule goes through
# %s rather than being a bare format string.
printf '\n%s\n' '----------------------------------------'
printf 'prepush summary\n'
printf '%s\n' '----------------------------------------'
for row in "${RESULTS[@]}"; do
  name="${row%%$'\t'*}"
  rest="${row#*$'\t'}"
  status="${rest%%$'\t'*}"
  note="${rest#*$'\t'}"
  case "$status" in
    PASS) colour=$'\033[32m' ;;
    FAIL) colour=$'\033[31m' ;;
    *)    colour=$'\033[33m' ;;
  esac
  printf '  %-*s  %s%s%s' "$WIDTH" "$name" "$colour" "$status" $'\033[0m'
  [ -n "$note" ] && printf '  (%s)' "$note"
  printf '\n'
done
printf '\n'
for row in "${RESULTS[@]}"; do
  rest="${row#*$'\t'}"
  printf '%s\n' "${rest%%$'\t'*}"
done | sort | uniq -c | while read -r count status; do
  printf '  %s: %s\n' "$status" "$count"
done
echo "  source: .github/workflows/ci.yml verify job"

if [ "$FAILED" -ne 0 ]; then
  printf '\nprepush: FAILED - do not push\n'
  exit 1
fi
printf '\nprepush: passed\n'
exit 0