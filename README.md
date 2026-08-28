# Pandora Agent

Pandora is a governed, CLI-first agent runtime built around:

```text
Parliament → Shadow Council → Harness → Gene → governed execution
```

The `ReferenceMonitor` is the sole authority that can issue effect permits. Genes request work; effect executors perform it only with a valid, scoped, one-shot permit.

Every effect request carries an immutable `ExecutionProfile` assembled before Parliament evaluates it. The profile binds the runtime, platform, policy version, workspace identity digest, containment snapshot, executor, and selected components. Its digest is part of the versioned operation-request digest, so a permit or receipt cannot be reused after the execution environment changes. The profile is evidence only; it cannot grant authority.

Lifecycle hooks are ordered declarative veto rules evaluated before effect
authorization. They may reduce authority, but cannot mutate requests, execute
code, resolve approvals, or issue permits. Runtime events remain the
observation surface.

## Status

The active prerelease is `2.0.0-beta.7` and is CLI-only. Existing legacy
preview tags remain immutable for compatibility. Release tags use plain
SemVer; prereleases use `alpha`, `beta`, and `rc` suffixes. Older codename tags
are historical references only. See [RELEASES.md](RELEASES.md),
[CHANGELOG.md](CHANGELOG.md), and [platform support](docs/PLATFORMS.md) for the
shipped scope and release gates.
The source tree also contains the production-readiness controls for the next
release: scoped identities, cryptographic device trust, encrypted secrets,
recovery archives, local crash records, and stable-release signing gates. See
[production readiness](docs/PRODUCTION.md).
For the remaining platform phases, including prompt caching, background and
parallel agents, evaluation-driven loops, memory consolidation, self-healing
tests, agent operations, and the OpenDesign-informed frontend direction, see
[the roadmap](docs/ROADMAP.md).

The npm package also exports a typed TypeScript client for the stable JSON CLI
contract. It forwards an argv array to the verified native binary and does not
create a second runtime or permission path. See [TypeScript client](docs/TYPESCRIPT.md).

For project context, contribution rules, and security reporting, see
[Why Pandora?](docs/WHY_PANDORA.md), [CONTRIBUTING.md](CONTRIBUTING.md), and
[SECURITY.md](SECURITY.md).

## Install and start

The bootstrap installers use the current published prerelease by default. They
verify the downloaded native binary against the release checksum manifest
before installation. Set `PANDORA_VERSION` to pin another published tag.

Install and open Pandora in one command:

```sh
curl -fsSL https://raw.githubusercontent.com/anisayakmitra-in/PANDORA-AGENT/main/scripts/install.sh | sh && "$HOME/.local/bin/pandora"
```

```powershell
irm https://raw.githubusercontent.com/anisayakmitra-in/PANDORA-AGENT/main/scripts/install.ps1 | iex; & "$env:LOCALAPPDATA\Pandora\bin\pandora.exe"
```

The first interactive launch creates the local configuration and opens the
Ratatui client. It asks only for provider metadata; API keys stay in the
environment. For scripted setup, use `pandora setup` instead.

To pin the current release explicitly:

```sh
curl -fsSL https://raw.githubusercontent.com/anisayakmitra-in/PANDORA-AGENT/main/scripts/install.sh | PANDORA_VERSION=v2.0.0-beta.7 sh
```

After installation, update to a specific published tag with the same checksum
verification:

```text
pandora update --release v2.0.0-beta.7
```

## Build

Requires Rust `1.97.1`.

```text
cargo test --workspace --lib --tests
cargo run -p pandora-cli -- --version
```

For scripted setup, use `pandora setup --interactive` or pass provider options
to `pandora setup`. In an interactive terminal, `pandora` starts the wizard
when configuration is missing, then opens the Ratatui client. Use `pandora chat`
for the line-oriented client and explicit subcommands for automation.

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

Local isolated subagents use the same governed runtime path with durable,
scoped records and exact-commit Git worktrees:

```text
pandora subagent spawn --session <id> --execution <id> "Review this workspace"
pandora subagent work --max-agents 2
pandora subagent inspect <subagent-id> --json
pandora subagent cleanup <subagent-id> --yes
```

`spawn` materializes the active provider and coding Harness when omitted.
`cancel` terminalizes queued work or requests cooperative cancellation for a
running child. Cleanup remains local, requires explicit confirmation, and
preserves dirty or commit-mismatched worktrees.

For a local client such as the future desktop app, start the authenticated
loopback runtime service:

```text
pandora service start --port 0
```

It prints one JSON readiness record with the bound endpoint and protected token
file path, then remains in the foreground until Ctrl-C. It never prints the
token or accepts non-loopback connections. See [CLI reference](docs/CLI.md).

The supported product target is the native CLI on Windows, macOS, and Linux. Desktop, remote execution, mobile, and package marketplace integration remain gated until their release tests and security boundaries exist. The CLI can manage profiles for the runtime's governed local stdio MCP preview and execute import-free WebAssembly package Genes through an admitted Domain Harness. See [MCP.md](docs/MCP.md) and [WebAssembly package Genes](docs/WASM.md).

For a clean-machine installation, release verification, and cross-platform
notes, see [Installation](docs/INSTALL.md). Published release artifacts are
the only supported installation source for end users; a local desktop build is
a development artifact until its release gates complete.

## License

Pandora Agent is released under the [MIT License](LICENSE).
