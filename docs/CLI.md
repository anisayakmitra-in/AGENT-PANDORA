# Pandora CLI

Status: Partial in the current `2.0.0-alpha.2` development line.

The CLI is the primary Pandora surface. Commands return versioned JSON with
`--json` and stable non-zero exit codes for usage, configuration, policy,
approval, execution, update, and internal failures.

## Setup and diagnostics

```text
pandora setup --provider-url https://provider.example/v1 --model gpt-5
pandora doctor --json
pandora provider list --json
pandora provider test --json
```

`doctor` reports the platform, CLI version, configuration path, storage path,
workspace path, policy mode, provider configuration state, and remediation.
Provider connectivity is deliberately `not_checked`; diagnostics do not send a
request or read a provider credential.

`provider test` sends one bounded request using the active profile's credential
environment variable and reports the selected model, response, and token usage.
Use `pandora provider set --provider-url <url> --model <model>` to configure the
backward-compatible `openai-compatible` profile. Multiple provider profiles can
keep separate endpoints, models, and credential variables:

```text
pandora provider set --name coding --provider-url https://coding.example/v1 --model coding-model --api-key-env PANDORA_CODING_API_KEY
pandora provider set --name design --provider-url https://design.example/v1 --model vision-model --api-key-env PANDORA_DESIGN_API_KEY
pandora provider use design
pandora provider list --json
pandora provider test --provider design --json
```

Use `pandora run --provider coding ...` for a one-run selection. Profiles store
only endpoint, model, and environment-variable names; Pandora never stores API
key values in configuration or output.

## Sessions and execution

```text
pandora run "read:README.md"
pandora run "search:needle"
pandora run --agent "Read the README and summarize it"
pandora run --harness coding --gene workspace.read "read:README.md"
pandora run --plan "inspect the README and report what it contains"
pandora run --approval <approval-id> "patch:README.md:approved content"
pandora session list
pandora session resume <session-id>
pandora approval list
pandora approval inspect <approval-id>
pandora approval resolve <approval-id> --allow
```

Read-only work can complete without approval. Writes and process effects stop at
the approval boundary and expose an inspectable, redacted request subject.
`--harness` selects an installed Harness by ID; `coding` is an alias for the
built-in `coding-domain` Harness. The runtime rejects an unknown or unsupported
Harness before Gene planning.
`search:<query>` scans regular files under the configured workspace within the
runtime's entry and file-size limits, does not follow symlinks, and returns
matching paths with forward-slash separators.
After an operator approves a write, rerun the exact task with its approval ID.
The approval is bound to the original session, execution, Gene, and request
digest, is consumed atomically, and cannot be replayed for another task.
`run --plan` sends the request to the active or explicitly selected provider as a bounded,
tool-free planning call. Only a schema-validated task intent is passed to the
runtime; the model cannot execute tools or grant permissions. Configure the
active profile with `pandora provider set` and provide the environment variable
named by that profile for planning.

`run --agent` enables the bounded multi-turn loop. The model can call
`workspace.read`, `workspace.search`, `workspace.patch`, and `workspace.verify`.
Each call is validated by the ToolEngine, routed through the same governed
runtime, and recorded in the session. Read and search use the current read-only
policy; patch and verify stop at the existing approval boundary before any
filesystem or process effect. The loop allows eight model turns and sixteen
tool calls by default. Set `--max-turns` and `--max-tools` to choose budgets for
a run; each value must be between 1 and 64 turns or 1 and 128 tool calls.
Agent mode cannot be combined with `--plan`, `--harness`, or `--gene`. User,
assistant, and tool messages are stored in
the scoped session database with fixed size limits; the system instruction is
rebuilt for every run. Reusing `--session <id>` restores that bounded
conversation before adding the new task. Credentials and hidden model
reasoning are not persisted. To continue an approved write, rerun
`run --agent --approval <id>` with the returned session ID and a follow-up task.
Pandora replays the bounded pending tool call through the same approval and
consumes it once.

## Discovery and completions

```text
pandora harness list
pandora tool list
pandora tool inspect <id>
pandora skill list
pandora skill inspect <id>
pandora skill enable <id>
pandora skill suspend <id>
pandora skill disable <id>
pandora skill remove <id> --dry-run
pandora skill remove <id> --yes
pandora skill restore <id>
pandora orchestration roles
pandora strategies list
pandora completions powershell
pandora completions bash
pandora completions zsh
pandora completions fish
```

Completion commands print a shell script. They describe the public command
surface and do not execute a command or inspect credentials.

`tool list` and `tool inspect` expose the built-in tool contract only: version,
name, required capability, operation, and input schema. They do not execute a
tool or bypass the governed execution path.

Skills are discovered from the configured data directory under `skills/`.
Use `--root <path>` to inspect another local skill root. Listing and inspection
read metadata, state, provenance, resources, and script inventory only; Skills
start disabled and scripts are not executed by these commands. `enable`,
`suspend`, and `disable` persist the explicit lifecycle state under the skill
root. `remove --dry-run` previews the skill path; `remove --yes` moves the
skill into the reversible removal area, and `restore` returns it as disabled.
These commands do not execute scripts; script execution remains available only
through the governed ToolEngine path.

## Configuration migration

```text
pandora migrate config --config <path>
pandora migrate config --config <path> --dry-run
```

Migration accepts the unversioned legacy fields `provider` (a URL string or an
object with `url`), `data_path`, and `workspace_path`, and writes the current
`format_version: 1` shape. A sibling `<path>.bak` backup is created before the
replacement. Existing backups are never overwritten, malformed input is left
untouched, and migration is one-way.

## Verified updates and uninstall

```text
pandora update --artifact <path> --sha256 sha256:<64-hex-digits>
pandora update --artifact <path> --sha256 sha256:<64-hex-digits> --dry-run
pandora update --rollback
pandora uninstall --dry-run
pandora uninstall --yes
```

Updates verify the complete local artifact before staging it under the Pandora
data directory. A detached Ed25519 signature can be checked with matching
`--public-key <64-hex-digits>` and `--signature <128-hex-digits>` options.
The previous staged artifact is retained for one-step rollback. No update
operation executes an unverified artifact.

Uninstall requires `--yes` for deletion, preserves the configured workspace,
and removes only the Pandora configuration file and data directory. Use
`--dry-run` to inspect the exact paths first.
