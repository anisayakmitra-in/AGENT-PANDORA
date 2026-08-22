# Pandora Agent

Pandora is a governed, CLI-first agent runtime built around:

```text
Parliament → Shadow Council → Harness → Gene → governed execution
```

The `ReferenceMonitor` is the sole authority that can issue effect permits. Genes request work; effect executors perform it only with a valid, scoped, one-shot permit.

Lifecycle hooks are ordered declarative veto rules evaluated before effect
authorization. They may reduce authority, but cannot mutate requests, execute
code, resolve approvals, or issue permits. Runtime events remain the
observation surface.

## Status

The active prerelease is `2.0.0-alpha.6` and is CLI-only. Existing legacy
preview tags remain immutable for compatibility. Release tags use plain
SemVer; prereleases use `alpha`, `beta`, and `rc` suffixes. Older codename tags
are historical references only. See [RELEASES.md](RELEASES.md),
[CHANGELOG.md](CHANGELOG.md), and [platform support](docs/PLATFORMS.md) for the
shipped scope and release gates.

## Install a tagged CLI release

The bootstrap installers use the current published prerelease by default. They
verify the downloaded native binary against the release checksum manifest
before installation. Set `PANDORA_VERSION` when you need to pin a different
published tag.

```sh
curl -fsSL https://raw.githubusercontent.com/anisayakmitra-in/PANDORA-AGENT/main/scripts/install.sh | sh
```

```powershell
irm https://raw.githubusercontent.com/anisayakmitra-in/PANDORA-AGENT/main/scripts/install.ps1 | iex
```

To pin the current release explicitly:

```sh
curl -fsSL https://raw.githubusercontent.com/anisayakmitra-in/PANDORA-AGENT/main/scripts/install.sh | PANDORA_VERSION=v2.0.0-alpha.6 sh
```

After installation, update to a specific published tag with the same checksum
verification:

```text
pandora update --release v2.0.0-alpha.6
```

## Build

Requires Rust `1.97.1`.

```text
cargo test --workspace --lib --tests
cargo run -p pandora-cli -- --version
```

For first-time local setup, use `pandora setup --interactive`. It records only
provider endpoint metadata and the name of the environment variable that holds
the credential; the credential itself stays outside Pandora configuration.

In an interactive terminal, running `pandora` with no arguments starts the
setup wizard when configuration is missing, then opens the Ratatui client. Use
`pandora chat` for the line-oriented client and explicit subcommands for
scripts and automation.

Headless workers can queue the same governed `run` command without a second
execution path:

```text
pandora job submit -- --agent "Review this workspace"
pandora job work
pandora job work --max-jobs 8
pandora job list --json
```

`job work` claims one queued record by default. `--max-jobs` processes up to 64
records in FIFO order, one at a time. The worker exits when it reaches the
limit, empties the queue, encounters an approval pause, or sees a failed run.
Each invocation records its worker ID and can finish only the jobs it claimed.
If a worker exits after a claim, Pandora leaves the job running and never
replays it automatically. After reviewing external effects, an operator can
record `pandora job mark-interrupted <job-id> --reason "..." --yes`.
Run it from an existing service manager or scheduler when continuous polling
is needed.

For a local client such as the future desktop app, start the authenticated
loopback runtime service:

```text
pandora service start --port 0
```

It prints one JSON readiness record with the bound endpoint and protected token
file path, then remains in the foreground until Ctrl-C. It never prints the
token or accepts non-loopback connections. See [CLI reference](docs/CLI.md).

The supported product target is the native CLI on Windows, macOS, and Linux. Desktop, remote execution, mobile, and package marketplace integration remain gated until their release tests and security boundaries exist.
