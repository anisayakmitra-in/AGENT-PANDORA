use super::{load_config, parse_options, write_config};
use crate::output::{CliError, CommandResult, success};
use pandora_provider::{ChatMessage, HttpProvider, ModelRequest, Provider, ProviderManifest};
use pandora_runtime::config::{
    DEFAULT_PROVIDER_API_KEY_ENV, DEFAULT_PROVIDER_NAME, ProviderProfile,
};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("provider requires 'list', 'set', 'use', or 'test'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "set" => set(&args[1..]),
        "use" => use_provider(&args[1..]),
        "test" => test(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown provider command '{unknown}'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let config = load_config(&parsed)?;
    let providers = config
        .provider_names()
        .into_iter()
        .map(|name| {
            let profile = config
                .provider_profile(&name)
                .expect("provider names must resolve to profiles");
            let manifest = ProviderManifest::new(
                profile.name(),
                profile.name(),
                profile.base_url(),
                profile.model(),
                profile.api_key_env(),
            )
            .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
            let mut value = serde_json::to_value(manifest).map_err(|_| {
                CliError::internal("could not serialize provider metadata", json!({}))
            })?;
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "active".to_owned(),
                    json!(config.active_provider() == Some(profile.name())),
                );
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, CliError>>()?;
    Ok(success(
        "provider list",
        json!({"providers": providers}),
        if providers.is_empty() {
            "No providers configured".to_owned()
        } else {
            format!("{} provider configured", providers.len())
        },
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
            "provider-url",
            "model",
            "api-key-env",
        ],
    )?;
    if parsed.value("provider-url").is_none() {
        return Err(CliError::usage(
            "provider set requires '--provider-url <url>'",
        ));
    }
    let mut config = load_config(&parsed)?;
    let name = parsed.value("name").unwrap_or(DEFAULT_PROVIDER_NAME);
    let model = parsed
        .value("model")
        .or_else(|| config.provider_profile(name).map(ProviderProfile::model))
        .unwrap_or("default");
    let api_key_env = parsed
        .value("api-key-env")
        .or_else(|| {
            config
                .provider_profile(name)
                .map(ProviderProfile::api_key_env)
        })
        .unwrap_or(DEFAULT_PROVIDER_API_KEY_ENV);
    let profile = ProviderProfile::new(
        name,
        parsed
            .value("provider-url")
            .expect("provider URL was checked"),
        model,
        api_key_env,
    )
    .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    config.set_provider_profile(profile);
    config
        .set_active_provider(name)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    write_config(&config)?;
    Ok(success(
        "provider set",
        json!({
            "provider": name,
            "base_url": config.provider_url(),
            "model": config.provider_model().unwrap_or("default"),
            "api_key_env": config.provider_api_key_env(),
            "active": config.active_provider() == Some(name),
        }),
        format!("Provider {name} configured"),
    ))
}

fn use_provider(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "provider use requires exactly one provider name",
        ));
    }
    let mut config = load_config(&parsed)?;
    let name = parsed.positionals[0].as_str();
    config
        .set_active_provider(name)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    write_config(&config)?;
    Ok(success(
        "provider use",
        json!({
            "provider": name,
            "model": config.provider_model().unwrap_or("default"),
            "api_key_env": config.provider_api_key_env(),
        }),
        format!("Provider {name} is active"),
    ))
}

fn test(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "provider", "model"],
    )?;
    let config = load_config(&parsed)?;
    super::require_config_file(&config)?;
    let base_url = config.provider_url().ok_or_else(|| {
        CliError::configuration(
            "provider is not configured; run 'pandora provider set' first",
            json!({"config_path": config.config_path()}),
        )
    })?;
    let model = parsed
        .value("model")
        .or(config.provider_model())
        .unwrap_or("default");
    let provider_name = config.active_provider().unwrap_or(DEFAULT_PROVIDER_NAME);
    let manifest = ProviderManifest::new(
        provider_name,
        provider_name,
        base_url,
        model,
        config
            .provider_api_key_env()
            .unwrap_or(DEFAULT_PROVIDER_API_KEY_ENV),
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let provider = HttpProvider::from_environment(manifest.clone())
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        vec![
            ChatMessage::user("Reply with exactly: ready")
                .map_err(|error| CliError::provider(error.to_string(), json!({})))?,
        ],
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let response = provider
        .complete(request)
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    Ok(success(
        "provider test",
        json!({
            "provider": manifest.id(),
            "model": manifest.default_model(),
            "status": "ready",
            "output": response.text(),
            "usage": {
                "prompt_tokens": response.usage().prompt_tokens(),
                "completion_tokens": response.usage().completion_tokens(),
                "total_tokens": response.usage().total_tokens(),
            },
        }),
        format!("Provider {} is ready", manifest.id()),
    ))
}
