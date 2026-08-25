# CLI JSON contract

Contract version: `0.1`

Pandora prints one UTF-8 JSON object to standard output when `--json` is
present. A successful command exits with `0`. An error exits with the numeric
code listed below. Human-readable output is not part of this contract.

## Success envelope

Every successful response contains:

| Field | Type | Meaning |
|---|---|---|
| `version` | string | JSON contract version. It is `0.1` for this contract. |
| `command` | string | Stable command identifier. |

Command data is added at the top level. Consumers should ignore unknown fields
so compatible releases can add evidence without renaming existing fields.

### Release commands

| Invocation | `command` | Required data |
|---|---|---|
| `pandora --version --json` | `version` | `pandora_version` |
| `pandora setup --json` | `setup` | `config_path`, `data_dir`, `workspace`, `provider_configured`, `provider_profiles`, `active_provider`, `provider_model`, `api_key_env`, `interactive` |
| `pandora doctor --json` | `doctor` | `healthy`, `version`, `platform`, `config_path`, `storage_path`, `workspace_path`, `provider`, `policy`, `containment`, `checks` |
| `pandora fleet list --json` | `fleet list` | `nodes`, `leases` |
| `pandora fleet dispatch <capability> --json` | `fleet dispatch` | `capability`, `node` |
| `pandora fleet lease <id> ... --json` | `fleet lease` | `lease` |
| `pandora update --artifact ... --json` | `update` | `verified`, `artifact`, `target`, `signature_verified`, `dry_run` |
| `pandora update --release ... --json` | `update` | `verified`, `release`, `artifact`, `signature_verified`, `dry_run`; non-dry-run responses also contain `target` |
| `pandora update --rollback --json` | `update rollback` | `target`, `dry_run`; a dry run contains `previous`, while a completed rollback contains `restored` |
| `pandora uninstall --dry-run --json` | `uninstall` | `dry_run`, `would_remove`, `preserved` |
| `pandora uninstall --yes --json` | `uninstall` | `dry_run`, `removed`, `preserved` |

Paths use the operating system's native string representation. `doctor`
reports evidence only; its containment object does not grant permissions or
claim that an executor is sandboxed.

## Error envelope

Every JSON error contains:

| Field | Type | Meaning |
|---|---|---|
| `version` | string | JSON contract version. |
| `code` | string | Stable symbolic error class. |
| `message` | string | Bounded operator-facing explanation. |
| `details` | object | Error-specific, machine-readable evidence. |

Errors do not contain a `command` field. They also do not contain a separate
numeric `exit_code` field; the process exit status is authoritative.

| Exit status | `code` |
|---:|---|
| `2` | `usage_error` |
| `10` | `configuration_error` |
| `20` | `provider_error` |
| `30` | `policy_denied` |
| `40` | `approval_required` |
| `50` | `execution_failed` |
| `60` | `internal_error` |
| `70` | `update_error` |

Example:

```json
{"version":"0.1","code":"usage_error","message":"update requires '--release <tag>', '--artifact <path>', or '--rollback'","details":{}}
```

Credential values, raw environment values, prompts, model output, and hidden
reasoning are not JSON contract fields. Responses may include the name of a
credential environment variable because it is configuration metadata, not the
credential itself.
