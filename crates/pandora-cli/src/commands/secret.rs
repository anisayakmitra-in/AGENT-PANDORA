use super::{load_config, parse_options, session_scope, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::SecretVault;
use serde_json::json;
use std::io::{self, Read};

const MASTER_KEY_ENV: &str = "PANDORA_MASTER_KEY";
const MAX_STDIN_BYTES: u64 = 64 * 1024 + 1;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("secret requires 'set', 'list', 'status', or 'remove'"))?;
    match subcommand.as_str() {
        "set" => set(&args[1..]),
        "list" => list(&args[1..]),
        "status" => status(&args[1..]),
        "remove" => remove(&args[1..]),
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
