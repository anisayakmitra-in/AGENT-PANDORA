# Pandora CLI

Status: Partial in Anubis `v2.0.x`.

The CLI is the primary Pandora surface. Commands return versioned JSON with
`--json` and stable non-zero exit codes for usage, configuration, policy,
approval, execution, update, and internal failures.

## Setup and diagnostics

```text
pandora setup --provider-url https://provider.example/v1
pandora doctor --json
pandora provider list --json
```

`doctor` reports the platform, CLI version, configuration path, storage path,
workspace path, policy mode, provider configuration state, and remediation.
Provider connectivity is deliberately `not_checked`; diagnostics do not send a
request or read a provider credential.

## Sessions and execution

```text
pandora run "read:README.md"
pandora session list
pandora session resume <session-id>
pandora approval list
pandora approval inspect <approval-id>
pandora approval resolve <approval-id> --allow
```

Read-only work can complete without approval. Writes and process effects stop at
the approval boundary and expose an inspectable, redacted request subject.

## Discovery and completions

```text
pandora harness list
pandora tool list
pandora orchestration roles
pandora strategies list
pandora completions powershell
pandora completions bash
pandora completions zsh
pandora completions fish
```

Completion commands print a shell script. They describe the public command
surface and do not execute a command or inspect credentials.

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
