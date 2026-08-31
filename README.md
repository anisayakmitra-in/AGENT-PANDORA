# Pandora

Pandora is a local-first agent platform with two interfaces: a Tauri desktop
app and the `pandora` CLI. Both use the same Rust runtime, authenticated
loopback service, and governed effect path:

```text
ExecutionController → Parliament → ReferenceMonitor → executor → receipt
```

Shadow Council selects an approved Harness, Gene, provider, and model
composition. It cannot grant capabilities, issue effect permits, or bypass
approvals. The `ReferenceMonitor` alone issues scoped, one-shot effect permits.

Every effect request carries an immutable `ExecutionProfile` assembled before Parliament evaluates it. The profile binds the runtime, platform, policy version, workspace identity digest, containment snapshot, executor, and selected components. Its digest is part of the versioned operation-request digest, so a permit or receipt cannot be reused after the execution environment changes. The profile is evidence only; it cannot grant authority.

Lifecycle hooks are ordered declarative veto rules evaluated before effect
authorization. They may reduce authority, but cannot mutate requests, execute
code, resolve approvals, or issue permits. Runtime events remain the
observation surface.

## Status

The active prerelease is `2.0.0-beta.7`. The repository now builds two local
product surfaces: the Rust CLI and a Tauri desktop app. Desktop release builds
bundle the same-commit CLI as a native sidecar and connect only to Pandora's
authenticated loopback service. The webview cannot issue permits or execute
tools by itself.

Existing legacy preview tags remain immutable for compatibility. Release tags
use plain SemVer; prereleases use `alpha`, `beta`, and `rc` suffixes. Older
codename tags are historical references only. See
[RELEASES.md](RELEASES.md), [CHANGELOG.md](CHANGELOG.md), and
[platform support](docs/PLATFORMS.md) for the shipped scope and release gates.
The source tree also contains the production-readiness controls for the next
release: scoped identities, cryptographic device trust, encrypted secrets,
recovery archives, local crash records, and stable-release signing gates. See
[production readiness](docs/PRODUCTION.md).
Pandora now binds local context caching and provider-native stable-prefix
caching to the same classification and provenance boundaries as execution. See
[prompt caching](docs/PROMPT_CACHING.md). Background and parallel agents,
evaluation primitives, scoped memory synthesis, and the desktop foundation are
also present. For the audited shipped/open split, see
[the roadmap](docs/ROADMAP.md).

Registered evaluation suites can run on durable worker-owned schedules.
Binding a staged proposal creates a one-shot canary that records the exact
report digest and case counts, then pauses at canary evidence; it never
activates the candidate. See [evaluation](docs/EVALUATION.md).

The npm package also exports a typed TypeScript client for the stable JSON CLI
contract. It forwards an argv array to the verified native binary and does not
create a second runtime or permission path. See [TypeScript client](docs/TYPESCRIPT.md).
Contributors can start a declarative Domain Harness with the local-only
[`sdk/domain-harness-starter`](sdk/domain-harness-starter/README.md) reference
package or `pandora package scaffold domain-harness --output <new-directory>`.
Composition authors can use the metadata-only
[`sdk/meta-harness-starter`](sdk/meta-harness-starter/README.md) reference or
`pandora package scaffold meta-harness --output <new-directory>`.
Gene authors can evaluate explicit no-effect, bounded-read, and approval-bound
effect proposals with the validated [`sdk/gene-pack`](sdk/gene-pack/README.md)
examples. The CLI, TUI, and desktop inspector expose their signed capability
contracts, provenance, owning Domain, lifecycle generation, and rollback state.

For project context, contribution rules, and security reporting, see
[Why Pandora?](docs/WHY_PANDORA.md), [CONTRIBUTING.md](CONTRIBUTING.md), and
[SECURITY.md](SECURITY.md).

## Install and start

### CLI

The bootstrap installers use the current published prerelease by default. They
verify the downloaded native binary against the release checksum manifest
before installation. Set `PANDORA_VERSION` to pin another published tag.

Install and open Pandora in one command:

```sh
curl -fsSL https://raw.githubusercontent.com/anisayakmitra-in/AGENT-PANDORA/main/scripts/install.sh | sh && "$HOME/.local/bin/pandora"
```

```powershell
irm https://raw.githubusercontent.com/anisayakmitra-in/AGENT-PANDORA/main/scripts/install.ps1 | iex; & "$env:LOCALAPPDATA\Pandora\bin\pandora.exe"
```

The first interactive launch creates the local configuration and opens the
Ratatui client. It asks only for provider metadata; API keys stay in the
environment. For scripted setup, use `pandora setup` instead.

To pin the current release explicitly:

```sh
curl -fsSL https://raw.githubusercontent.com/anisayakmitra-in/AGENT-PANDORA/main/scripts/install.sh | PANDORA_VERSION=v2.0.0-beta.7 sh
```

After installation, update to a specific published tag with the same checksum
verification:

```text
pandora update --release v2.0.0-beta.7
```

### Desktop app

Pandora Desktop is local and has no account or login screen. Its package
contains the same-commit `pandora` CLI sidecar, so launching the app does not
depend on an inherited shell or `PATH`.

Build a local package from source:

```sh
cd apps/pandora-desktop
npm ci
npm run tauri:build
```

On macOS, `./script/build_and_run.sh --verify` builds the app bundle and runs
the project checks. See [Pandora Desktop](apps/pandora-desktop/README.md) for
platform behavior and direct-distribution limits.

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

Durable Meta-Harness orchestration workers can coordinate bounded roles across
explicit repository and workspace identities:

```text
pandora orchestration submit --input plan.json
pandora orchestration claim --worker worker-a --json
pandora orchestration complete <run-id> --worker worker-a --role <role-id> --receipt receipt.json
```

Assignments remain coordination evidence; workers execute them through the
existing governed run or subagent path. See [durable orchestration workers](docs/ORCHESTRATION.md).

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
running child. If the request lands after the last checkpoint but before
terminal storage, the durable store still records cancellation; a late provider
response cannot overwrite it with success. Cleanup remains local, requires
explicit confirmation, and preserves dirty or commit-mismatched worktrees.

For the native desktop development build or another local client, start the
authenticated loopback runtime service:

```text
pandora service start --port 0
```

It prints one JSON readiness record with the bound endpoint and protected token
file path, then remains in the foreground until Ctrl-C. It never prints the
token or accepts non-loopback connections. See [CLI reference](docs/CLI.md).

The `2.0.0-beta.7` installers remain CLI-only because that tag predates the
desktop packaging work. The current main branch builds the desktop on Ubuntu,
Windows, and macOS 26 CI. A stable desktop support claim still requires signed
and notarized packages, clean-machine install and update drills, and retained
release evidence. Remote execution, mobile, and a public package marketplace
remain outside the shipped boundary.

The CLI and desktop use the same governed local stdio MCP preview and admitted
WebAssembly package Genes. See [MCP.md](docs/MCP.md),
[WebAssembly package Genes](docs/WASM.md), and
[Installation](docs/INSTALL.md).

## License

Pandora is released under the [Apache License 2.0](LICENSE).
