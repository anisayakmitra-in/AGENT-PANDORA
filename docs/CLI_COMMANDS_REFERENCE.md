# Pandora CLI Commands Reference

Complete command and flag reference for the `pandora` binary.

---

## Root Commands Index

| Command | Summary | Key Subcommands |
|---|---|---|
| `pandora` | Interactive TUI dashboard | — |
| `pandora setup` | Onboarding setup wizard & provider initialization | `--provider`, `--model`, `--api-key` |
| `pandora chat` | Interactive REPL chat session | `--model`, `--mode`, `--provider` |
| `pandora run` | Execute task (agent, plan, or evaluation) | `--agent`, `--max-turns`, `--mode`, `--json` |
| `pandora prompt` | Declarative reusable prompt presets | `save`, `list`, `inspect`, `apply`, `delete` |
| `pandora mcp` | Model Context Protocol server management | `list`, `inspect`, `add`, `add-http`, `login`, `logout`, `remove`, `catalog`, `call` |
| `pandora provider` | Model provider configuration & testing | `list`, `set`, `add`, `remove`, `use`, `login`, `accounts`, `test` |
| `pandora session` | Durable session inspection & resumption | `list`, `resume`, `inspect`, `events` |
| `pandora secret` | Encrypted SecretVault credentials | `set`, `list`, `status`, `remove` |
| `pandora cron` | Autonomous background task scheduling | `create`, `list`, `inspect`, `disable`, `remove`, `run` |
| `pandora memory` | Multi-layered memory operations | `recall`, `audit`, `forget`, `compact`, `promote`, `synthesize`, `schedule`, `vfs` |
| `pandora skill` | Procedural skill management | `list`, `inspect`, `install`, `enable`, `disable`, `remove` |
| `pandora package` | WebAssembly Gene package management | `scaffold`, `admit`, `validate`, `sign`, `install`, `lock` |
| `pandora evaluation` | Autonomous evaluation scorecards | `golden`, `inspect`, `scorecard`, `suite`, `regression`, `schedule` |
| `pandora evolution` | Self-improving model & strategy proposals | `generate`, `list`, `inspect`, `submit`, `evaluate`, `approve`, `rollout` |
| `pandora fleet` | Multi-node cluster orchestration | `dashboard`, `list`, `register`, `dispatch`, `supervisor` |
| `pandora doctor` | System diagnostic health check | `--fix` |
| `pandora policy` | Inspect active reference monitor policy | `--export` |
| `pandora update` | In-place self-update with signature verification | `--channel`, `--version` |
| `pandora completions` | Generate shell autocompletion scripts | `powershell`, `bash`, `zsh`, `fish` |

---

## Global Options
- `--config <path>`: Override path to `config.json` (defaults to `$PANDORA_CONFIG` or `.pandora/config.json`).
- `--data-dir <path>`: Override state database directory (defaults to `$PANDORA_DATA_DIR` or `.pandora/data`).
- `--workspace <path>`: Target workspace directory (defaults to current working directory).
- `--json`: Format command output as machine-readable JSON.
- `--help`: Display synopsis, subcommands, and flags.
- `--version`: Display binary version.
