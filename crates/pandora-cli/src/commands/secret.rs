use super::{load_config, parse_options, session_scope, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::SecretVault;
use serde_json::json;
use std::io::{self, Read};

use pandora_types::{CredentialPool, PoolStrategy};

const MASTER_KEY_ENV: &str = "PANDORA_MASTER_KEY";
const MAX_STDIN_BYTES: u64 = 64 * 1024 + 1;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("secret requires 'set', 'list', 'status', 'remove', or 'pool'")
    })?;
    match subcommand.as_str() {
        "set" => set(&args[1..]),
        "list" => list(&args[1..]),
        "status" => status(&args[1..]),
        "remove" => remove(&args[1..]),
        "pool" => pool(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown secret command '{unknown}'"
        ))),
    }
}

fn set(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "value-stdin"])?;
    if parsed.positionals.len() != 1 || parsed.value("value-stdin").is_none() {
        return Err(CliError::usage(
            "secret set requires '<ENV_NAME> --value-stdin'",
        ));
    }
    let name = &parsed.positionals[0];
    let config = load_config(&parsed)?;
    let mut value = String::new();
    io::stdin()
        .take(MAX_STDIN_BYTES)
        .read_to_string(&mut value)
        .map_err(|_| CliError::configuration("could not read secret from stdin", json!({})))?;
    while value.ends_with(['\r', '\n']) {
        value.pop();
    }
    if value.len() >= MAX_STDIN_BYTES as usize {
        return Err(CliError::configuration(
            "secret exceeds the size limit",
            json!({}),
        ));
    }
    let mut vault = open_vault(&config)?;
    let entry = vault
        .put(name, value, timestamp().as_unix_seconds())
        .map_err(vault_error)?;
    Ok(success(
        "secret set",
        json!({
            "name": entry.name(),
            "stored": true,
            "vault_path": vault.path(),
            "created_at": entry.created_at(),
            "updated_at": entry.updated_at(),
        }),
        format!("Encrypted secret {} stored", entry.name()),
    ))
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "secret list accepts no positional arguments",
        ));
    }
    let config = load_config(&parsed)?;
    let vault = open_vault(&config)?;
    let entries = vault.list();
    Ok(success(
        "secret list",
        json!({
            "secrets": entries,
            "count": entries.len(),
            "values_exposed": false,
        }),
        format!("{} encrypted secret reference(s)", entries.len()),
    ))
}

fn status(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage("secret status requires '<ENV_NAME>'"));
    }
    let name = &parsed.positionals[0];
    let config = load_config(&parsed)?;
    let vault = open_vault(&config)?;
    let configured = vault.get(name).map_err(vault_error)?.is_some();
    Ok(success(
        "secret status",
        json!({
            "name": name,
            "configured": configured,
            "value_exposed": false,
        }),
        if configured {
            format!("Encrypted secret {name} is configured")
        } else {
            format!("Encrypted secret {name} is not configured")
        },
    ))
}

fn remove(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "yes"])?;
    if parsed.positionals.len() != 1 || parsed.value("yes").is_none() {
        return Err(CliError::usage("secret remove requires '<ENV_NAME> --yes'"));
    }
    let name = &parsed.positionals[0];
    let config = load_config(&parsed)?;
    let mut vault = open_vault(&config)?;
    let removed = vault.remove(name).map_err(vault_error)?;
    Ok(success(
        "secret remove",
        json!({"name": name, "removed": removed}),
        if removed {
            format!("Encrypted secret {name} removed")
        } else {
            format!("Encrypted secret {name} was not present")
        },
    ))
}

fn pool(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "secret pool requires 'add', 'list', 'status', 'rotate', 'report-429', or 'remove'",
        )
    })?;
    match subcommand.as_str() {
        "add" => pool_add(&args[1..]),
        "list" => pool_list(&args[1..]),
        "status" => pool_status(&args[1..]),
        "rotate" => pool_rotate(&args[1..]),
        "report-429" => pool_report_429(&args[1..]),
        "remove" => pool_remove(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown secret pool command '{unknown}'"
        ))),
    }
}

fn pool_add(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "key",
            "key-id",
            "priority",
            "strategy",
            "cooldown",
        ],
    )?;
    let pool_id = parsed.positionals.first().ok_or_else(|| {
        CliError::usage("secret pool add requires '<POOL_ID> --key <SECRET_NAME>'")
    })?;
    let secret_name = parsed
        .value("key")
        .ok_or_else(|| CliError::usage("secret pool add requires '--key <SECRET_NAME>'"))?;
    let key_id = parsed.value("key-id").unwrap_or(secret_name);
    let priority = parsed
        .value("priority")
        .map(|v| {
            v.parse::<u32>().map_err(|_| {
                CliError::usage("secret pool add --priority must be a positive integer")
            })
        })
        .transpose()?
        .unwrap_or(1);
    let strategy = parsed
        .value("strategy")
        .map(|v| {
            PoolStrategy::parse(v).ok_or_else(|| {
                CliError::usage(format!(
                    "unknown pool strategy '{v}'; choose round-robin, least-used, or priority"
                ))
            })
        })
        .transpose()?
        .unwrap_or(PoolStrategy::PriorityFailover);
    let cooldown = parsed
        .value("cooldown")
        .map(|v| {
            v.parse::<u64>().map_err(|_| {
                CliError::usage("secret pool add --cooldown must be a positive integer")
            })
        })
        .transpose()?
        .unwrap_or(60);

    let config = load_config(&parsed)?;
    let mut vault = open_vault(&config)?;
    let mut pool = vault
        .get_pool(pool_id)
        .cloned()
        .unwrap_or_else(|| CredentialPool::new(pool_id, strategy));
    pool.strategy = strategy;
    pool.default_cooldown_seconds = cooldown;
    pool.add_or_update_key(key_id, secret_name, priority);
    vault.put_pool(pool.clone()).map_err(vault_error)?;

    Ok(success(
        "secret pool add",
        json!({
            "pool_id": pool_id,
            "key_id": key_id,
            "secret_name": secret_name,
            "priority": priority,
            "strategy": strategy.as_str(),
            "cooldown": cooldown,
            "total_keys": pool.keys.len(),
        }),
        format!("Added key {key_id} to credential pool {pool_id}"),
    ))
}

fn pool_list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "pool"])?;
    let config = load_config(&parsed)?;
    let vault = open_vault(&config)?;

    if let Some(pool_id) = parsed
        .value("pool")
        .or_else(|| parsed.positionals.first().map(|s| s.as_str()))
    {
        let pool = vault
            .get_pool(pool_id)
            .ok_or_else(|| CliError::usage(format!("unknown credential pool '{pool_id}'")))?;
        return Ok(success(
            "secret pool list",
            json!({
                "pool": pool,
            }),
            format!("Inspected credential pool {pool_id}"),
        ));
    }

    let pools = vault.list_pools();
    Ok(success(
        "secret pool list",
        json!({
            "pools": pools,
            "count": pools.len(),
        }),
        format!("{} credential pool(s) configured", pools.len()),
    ))
}

fn pool_status(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let pool_id = parsed
        .positionals
        .first()
        .ok_or_else(|| CliError::usage("secret pool status requires '<POOL_ID>'"))?;
    let config = load_config(&parsed)?;
    let vault = open_vault(&config)?;
    let pool = vault
        .get_pool(pool_id)
        .ok_or_else(|| CliError::usage(format!("unknown credential pool '{pool_id}'")))?;
    let status = pool.status(timestamp().as_unix_seconds());
    Ok(success(
        "secret pool status",
        json!({
            "status": status,
        }),
        format!(
            "Credential pool {} has {} active key(s) ({} in cooldown)",
            pool_id, status.active_keys, status.cooldown_keys
        ),
    ))
}

fn pool_rotate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let pool_id = parsed
        .positionals
        .first()
        .ok_or_else(|| CliError::usage("secret pool rotate requires '<POOL_ID>'"))?;
    let config = load_config(&parsed)?;
    let mut vault = open_vault(&config)?;
    let now = timestamp().as_unix_seconds();
    let mut pool = vault
        .get_pool(pool_id)
        .cloned()
        .ok_or_else(|| CliError::usage(format!("unknown credential pool '{pool_id}'")))?;
    let selected = pool.select_key(now).cloned().ok_or_else(|| {
        CliError::execution(
            format!("all keys in credential pool '{pool_id}' are in cooldown"),
            json!({}),
        )
    })?;
    vault.put_pool(pool).map_err(vault_error)?;
    Ok(success(
        "secret pool rotate",
        json!({
            "pool_id": pool_id,
            "selected_key": selected,
        }),
        format!(
            "Rotated credential pool {pool_id} to key {}",
            selected.key_id
        ),
    ))
}

fn pool_report_429(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "key", "cooldown"],
    )?;
    let pool_id = parsed.positionals.first().ok_or_else(|| {
        CliError::usage("secret pool report-429 requires '<POOL_ID> --key <KEY_ID>'")
    })?;
    let key_id = parsed
        .value("key")
        .ok_or_else(|| CliError::usage("secret pool report-429 requires '--key <KEY_ID>'"))?;
    let cooldown = parsed
        .value("cooldown")
        .map(|v| {
            v.parse::<u64>().map_err(|_| {
                CliError::usage("secret pool report-429 --cooldown must be an integer")
            })
        })
        .transpose()?;
    let config = load_config(&parsed)?;
    let mut vault = open_vault(&config)?;
    let now = timestamp().as_unix_seconds();
    let mut pool = vault
        .get_pool(pool_id)
        .cloned()
        .ok_or_else(|| CliError::usage(format!("unknown credential pool '{pool_id}'")))?;
    let next_reset = pool.record_rate_limit(key_id, cooldown, now);
    vault.put_pool(pool).map_err(vault_error)?;
    Ok(success(
        "secret pool report-429",
        json!({
            "pool_id": pool_id,
            "key_id": key_id,
            "next_reset_seconds": next_reset,
        }),
        format!("Recorded rate limit for key {key_id} in pool {pool_id}"),
    ))
}

fn pool_remove(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "key", "yes"])?;
    if parsed.value("yes").is_none() {
        return Err(CliError::usage("secret pool remove requires '--yes'"));
    }
    let pool_id = parsed
        .positionals
        .first()
        .ok_or_else(|| CliError::usage("secret pool remove requires '<POOL_ID> --yes'"))?;
    let config = load_config(&parsed)?;
    let mut vault = open_vault(&config)?;

    if let Some(key_id) = parsed.value("key") {
        let mut pool = vault
            .get_pool(pool_id)
            .cloned()
            .ok_or_else(|| CliError::usage(format!("unknown credential pool '{pool_id}'")))?;
        let removed = pool.remove_key(key_id);
        vault.put_pool(pool).map_err(vault_error)?;
        return Ok(success(
            "secret pool remove",
            json!({
                "pool_id": pool_id,
                "key_id": key_id,
                "removed": removed,
            }),
            if removed {
                format!("Removed key {key_id} from pool {pool_id}")
            } else {
                format!("Key {key_id} was not present in pool {pool_id}")
            },
        ));
    }

    let removed = vault.remove_pool(pool_id).map_err(vault_error)?;
    Ok(success(
        "secret pool remove",
        json!({
            "pool_id": pool_id,
            "removed": removed,
        }),
        if removed {
            format!("Credential pool {pool_id} removed")
        } else {
            format!("Credential pool {pool_id} was not present")
        },
    ))
}

pub(crate) fn open_vault(
    config: &pandora_runtime::config::RuntimeConfig,
) -> Result<SecretVault, CliError> {
    let passphrase = std::env::var(MASTER_KEY_ENV).map_err(|_| {
        CliError::configuration(
            "encrypted secrets require PANDORA_MASTER_KEY",
            json!({"environment": MASTER_KEY_ENV}),
        )
    })?;
    let (_, tenant, workspace) = session_scope();
    SecretVault::open(config.data_dir(), tenant, workspace, passphrase).map_err(vault_error)
}

pub(crate) fn vault_error(error: pandora_runtime::SecretVaultError) -> CliError {
    CliError::configuration(error.to_string(), json!({}))
}
