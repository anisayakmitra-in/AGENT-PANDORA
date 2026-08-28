use super::{load_config, parse_options, write_config};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::config::RegistryProfile;
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("registry requires 'list', 'set', 'use', or 'remove'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "set" => set(&args[1..]),
        "use" => use_registry(&args[1..]),
        "remove" => remove(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown registry command '{unknown}'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "registry list does not accept positional arguments",
        ));
    }
    let config = load_config(&parsed)?;
    let registries = config
        .registry_names()
        .into_iter()
        .map(|name| {
            let profile = config
                .registry_profile(&name)
                .expect("registry names must resolve to profiles");
            json!({
                "name": profile.name(),
                "base_url": profile.base_url(),
                "token_env": profile.token_env(),
                "active": config.active_registry() == Some(profile.name()),
            })
        })
        .collect::<Vec<_>>();
    Ok(success(
        "registry list",
        json!({"registries": registries}),
        format!("{} registry profile(s) configured", registries.len()),
    ))
}

fn set(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "name",
            "registry-url",
            "token-env",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "registry set does not accept positional arguments",
        ));
    }
    let name = parsed
        .value("name")
        .ok_or_else(|| CliError::usage("registry set requires '--name <name>'"))?;
    let base_url = parsed
        .value("registry-url")
        .ok_or_else(|| CliError::usage("registry set requires '--registry-url <url>'"))?;
    let token_env = parsed.value("token-env").map(str::to_owned);
    let profile = RegistryProfile::new(name, base_url, token_env)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    let mut config = load_config(&parsed)?;
    config.set_registry_profile(profile);
    config
        .set_active_registry(name)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    write_config(&config)?;
    Ok(success(
        "registry set",
        json!({
            "registry": {
                "name": name,
                "base_url": base_url.trim_end_matches('/'),
                "token_env": parsed.value("token-env"),
                "active": true,
            }
        }),
        format!("Registry {name} configured"),
    ))
}

fn use_registry(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "registry use requires exactly one registry name",
        ));
    }
    let name = parsed.positionals[0].as_str();
    let mut config = load_config(&parsed)?;
    config
        .set_active_registry(name)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    write_config(&config)?;
    let profile = config
        .registry_profile(name)
        .expect("active registry must resolve");
    Ok(success(
        "registry use",
        json!({
            "registry": {
                "name": profile.name(),
                "base_url": profile.base_url(),
                "token_env": profile.token_env(),
                "active": true,
            }
        }),
        format!("Registry {name} is active"),
    ))
}

fn remove(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "yes"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "registry remove requires exactly one registry name",
        ));
    }
    if parsed.value("yes").is_none() {
        return Err(CliError::usage("registry remove requires '--yes'"));
    }
    let name = parsed.positionals[0].as_str();
    let mut config = load_config(&parsed)?;
    if !config.remove_registry_profile(name) {
        return Err(CliError::configuration(
            "registry is not configured",
            json!({"registry": name}),
        ));
    }
    write_config(&config)?;
    Ok(success(
        "registry remove",
        json!({
            "registry": {
                "name": name,
                "state": "removed",
            },
            "active_registry": config.active_registry(),
        }),
        format!("Registry {name} removed"),
    ))
}
