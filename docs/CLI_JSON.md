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
| `pandora provider test --json` | `provider test` | `provider`, `model`, `status`, `output`, `usage`, `metrics` |
| `pandora auth enroll ... --json` | `auth enroll` | `identity`, `token_path`, `device_key_path`, `token_exposed`, `private_key_exposed` |
| `pandora secret list --json` | `secret list` | `secrets`, `count`, `values_exposed` |
| `pandora backup create ... --json` | `backup create` | `output`, `entries`, `encrypted`, `format_version` |
| `pandora backup inspect ... --json` | `backup inspect` | `input`, `created_at`, `entries`, `authenticated`, `paths_exposed` |
| `pandora backup restore ... --json` | `backup restore` | `input`, `restored_entries`, `pre_restore_backup`, `authenticated` |
| `pandora run <task> --json` | `run` | `session_id`, `execution_id`, `harness_id`, `gene_id`, `status`, `elapsed_ms`, `provider_metrics`, `evaluation` |
| `pandora doctor --json` | `doctor` | `healthy`, `version`, `platform`, `config_path`, `storage_path`, `workspace_path`, `provider`, `policy`, `containment`, `checks` |
| `pandora fleet list --json` | `fleet list` | `nodes`, `leases` |
| `pandora fleet dispatch <capability> --json` | `fleet dispatch` | `capability`, `node` |
| `pandora fleet lease <id> ... --json` | `fleet lease` | `lease` |
| `pandora package validate --manifest <path> --artifact <path> --json` | `package validate` | `valid`, `package`, `execution_boundary`, `persisted` |
| `pandora memory recall ... --json` | `memory recall` | `scope`, `tier`, `records`, `count`, `limit`, `durability` |
| `pandora memory audit ... --json` | `memory audit` | `scope`, `entries`, `count`, `durability` |
| `pandora memory forget ... --json` | `memory forget` | `dry_run`, `memory_id`, `scope`, `revoked` or `would_revoke` |
| `pandora memory promote ... --json` | `memory promote` | `promoted`, `approval_id`, `approval_consumed` |
| `pandora evaluation golden --input <path> --json` | `evaluation golden` | `total`, `passed`, `failed`, `digest`, `cases` |
| `pandora evolution evaluate --id <proposal-id> --input <path> --json` | `evolution evaluate` | `proposal_id`, `total`, `passed`, `failed`, `trajectory_score`, `outcome_score`, `holdout_passed`, `policy_passed`, `regression_passed`, `digest`, `cases`, `durability` |
| `pandora evolution submit --input <path> --json` | `evolution submit` | `proposal_id`, `state`, `durability` |
| `pandora evolution approve --input <path> --json` | `evolution approve` | `proposal_id`, `state`, `approver`, `signer`, `durability` |
| `pandora evolution stage --id <proposal-id> --json` | `evolution stage` | `proposal_id`, `state`, `durability` |
| `pandora evolution generate --session <id> --kind <kind> --target-id <id> --base <path> --output <path> --json` | `evolution generate` | `proposal_id`, `state`, `kind`, `target_id`, `base_artifact`, `candidate_artifact`, `evidence_digest`, `provider`, `output`, `runtime_authority_changed`, `next_required`, `durability` |
| `pandora evolution canary --input <path> --json` | `evolution canary` | `proposal_id`, `state`, `passed`, `failure_count`, `durability` |
| `pandora evolution activate --id <proposal-id> --json` | `evolution activate` | `proposal_id`, `state`, `base_artifact`, `candidate_artifact`, `activated_at`, `activation_scope`, `runtime_authority_changed`, `durability` |
| `pandora evolution rollback --id <proposal-id> --reason <text> --json` | `evolution rollback` | `proposal_id`, `state`, `restored_artifact`, `rolled_back_at`, `reason`, `durability` |
| `pandora evaluation inspect --session <id> --json` | `evaluation inspect` | `session_id`, `execution_id`, `count`, `result_counts`, `receipts`, `durability` |
| `pandora graph <kind> --input <path> [--store <path>] --json` | `graph build` | `kind`, `scope`, `source_count`, `nodes`, `edges`, `digest`, optional `persisted` |
| `pandora update --artifact ... --json` | `update` | `verified`, `artifact`, `target`, `signature_verified`, `dry_run` |
| `pandora update --release ... --json` | `update` | `verified`, `release`, `channel`, `artifact`, `signature_verified`, `dry_run`; non-dry-run responses also contain `target` |
| `pandora update --rollback --json` | `update rollback` | `target`, `dry_run`; a dry run contains `previous`, while a completed rollback contains `restored` |
| `pandora uninstall --dry-run --json` | `uninstall` | `dry_run`, `would_remove`, `preserved` |
| `pandora uninstall --yes --json` | `uninstall` | `dry_run`, `removed`, `preserved` |

Paths use the operating system's native string representation. `doctor`
reports evidence only; its containment object does not grant permissions or
claim that an executor is sandboxed.

Memory records returned by `memory recall` include `origin` (`explicit` or
`synthesized`) and `evidence_ids`. Synthesized records are L1 candidates only;
their evidence is descriptive provenance and does not authorize tools, policy,
package activation, or promotion.

The `provider test` `metrics` object contains `elapsed_ms`, `input_tokens`,
`output_tokens`, and `succeeded`. These values describe the completed provider
call and can be recorded alongside the existing response.

Successful direct and agent runs include `elapsed_ms`, the measured wall-clock
duration for the run. It is diagnostic evidence and does not include provider
credentials or model content.

Custom Wasm direct runs also include `artifact_resolution`. It reports the
base and resolved artifact hashes, whether a replacement was active,
`snapshot: "execution_profile"`, and `runtime_authority_changed: false`.

Agent runs also include `provider_metrics`, an ordered array with one entry per
provider attempt. Each entry reports the provider and model IDs, elapsed
milliseconds, input and output token counts, and whether the attempt succeeded.
Fallback attempts appear in execution order; provider receipts remain the
authority for the full effect history.

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
