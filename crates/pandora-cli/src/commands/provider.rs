use super::{load_config, parse_options, timestamp, write_config};
use crate::output::{CliError, CommandResult, success};
use pandora_provider::{
    ChatMessage, FailoverProvider, HttpProvider, ModelRequest, Provider, ProviderManifest,
    ProviderProtocol,
};
use pandora_runtime::ExecutionController;
use pandora_runtime::config::{
    DEFAULT_PROVIDER_API_KEY_ENV, DEFAULT_PROVIDER_NAME, ProviderPricing, ProviderProfile,
    RuntimeConfig,
};
use pandora_runtime::executors::WorkspaceRoot;
use pandora_types::{Capability, PolicyContext, Session, SessionId};
use serde_json::json;

pub(crate) fn configured_provider(
    config: &RuntimeConfig,
    model: &str,
    operation: &str,
) -> Result<Box<dyn Provider>, CliError> {
    configured_provider_for(config, model, operation, None)
}

pub(crate) fn configured_provider_for(
    config: &RuntimeConfig,
    model: &str,
    operation: &str,
    provider_override: Option<&str>,
) -> Result<Box<dyn Provider>, CliError> {
    let provider_name = provider_override
        .or(config.active_provider())
        .unwrap_or(DEFAULT_PROVIDER_NAME);
    let selected_profile = config.provider_profile(provider_name);
    if provider_override.is_some() && selected_profile.is_none() {
        return Err(CliError::configuration(
            "provider is not configured",
            json!({"provider": provider_name}),
        ));
    }
    let base_url = selected_profile
        .map(ProviderProfile::base_url)
        .or_else(|| config.provider_url())
        .ok_or_else(|| {
            CliError::configuration(
                format!(
                    "{operation} requires a configured provider; run 'pandora provider set' first"
                ),
                json!({"config_path": config.config_path()}),
            )
        })?;
    let api_key_env = selected_profile
        .map(ProviderProfile::api_key_env)
        .or_else(|| config.provider_api_key_env())
        .unwrap_or(DEFAULT_PROVIDER_API_KEY_ENV);
    let protocol = selected_profile
        .map(ProviderProfile::protocol)
        .unwrap_or_default();
    let manifest = ProviderManifest::new_with_protocol(
        provider_name,
        provider_name,
        protocol,
        base_url,
        model,
        api_key_env,
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let primary = HttpProvider::from_environment(manifest)
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?;

    let Some(fallback_name) = config
        .provider_profile(provider_name)
        .and_then(ProviderProfile::fallback_provider)
    else {
        return Ok(Box::new(primary));
    };
    let fallback_profile = config
        .provider_profile(fallback_name)
        .ok_or_else(|| CliError::configuration("provider is not configured", json!({})))?;
    let fallback_manifest = ProviderManifest::new_with_protocol(
        fallback_profile.name(),
        fallback_profile.name(),
        fallback_profile.protocol(),
        fallback_profile.base_url(),
        fallback_profile.model(),
        fallback_profile.api_key_env(),
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let fallback = HttpProvider::from_environment(fallback_manifest)
        .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    Ok(Box::new(FailoverProvider::new(
        Box::new(primary),
        Box::new(fallback),
    )))
}

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
            let manifest = ProviderManifest::new_with_protocol(
                profile.name(),
                profile.name(),
                profile.protocol(),
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
                object.insert(
                    "fallback_provider".to_owned(),
                    json!(profile.fallback_provider()),
                );
                object.insert(
                    "pricing".to_owned(),
                    json!(profile.pricing().map(|pricing| {
                        json!({
                            "input_micros_per_million_tokens": pricing
                                .input_micros_per_million_tokens(),
                            "output_micros_per_million_tokens": pricing
                                .output_micros_per_million_tokens(),
                        })
                    })),
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
            "protocol",
            "provider-url",
            "model",
            "api-key-env",
            "fallback-provider",
            "input-micros-per-million-tokens",
            "output-micros-per-million-tokens",
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
    let protocol = parsed
        .value("protocol")
        .map(str::parse::<ProviderProtocol>)
        .transpose()
        .map_err(|error| CliError::usage(error.to_string()))?
        .or_else(|| config.provider_profile(name).map(ProviderProfile::protocol))
        .unwrap_or_default();
    let fallback_provider = parsed.value("fallback-provider").or_else(|| {
        config
            .provider_profile(name)
            .and_then(ProviderProfile::fallback_provider)
    });
    let input_pricing = parsed
        .value("input-micros-per-million-tokens")
        .map(|value| parse_pricing(value, "input-micros-per-million-tokens"))
        .transpose()?;
    let output_pricing = parsed
        .value("output-micros-per-million-tokens")
        .map(|value| parse_pricing(value, "output-micros-per-million-tokens"))
        .transpose()?;
    let pricing = match (input_pricing, output_pricing) {
        (None, None) => config
            .provider_profile(name)
            .and_then(ProviderProfile::pricing),
        (Some(input), Some(output)) => Some(ProviderPricing::new(input, output)),
        _ => {
            return Err(CliError::usage(
                "provider pricing requires both input and output token rates",
            ));
        }
    };
    let mut profile = ProviderProfile::new_with_protocol(
        name,
        protocol,
        parsed
            .value("provider-url")
            .expect("provider URL was checked"),
        model,
        api_key_env,
    )
    .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    if let Some(fallback_provider) = fallback_provider {
        profile = profile
            .with_fallback_provider(fallback_provider)
            .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    }
    if let Some(pricing) = pricing {
        profile = profile.with_pricing(pricing);
    }
    config.set_provider_profile(profile);
    config
        .set_active_provider(name)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    write_config(&config)?;
    Ok(success(
        "provider set",
        json!({
            "provider": name,
            "protocol": protocol,
            "base_url": config.provider_url(),
            "model": config.provider_model().unwrap_or("default"),
            "api_key_env": config.provider_api_key_env(),
            "fallback_provider": config
                .provider_profile(name)
                .and_then(ProviderProfile::fallback_provider),
            "pricing": config
                .provider_profile(name)
                .and_then(ProviderProfile::pricing)
                .map(|pricing| json!({
                    "input_micros_per_million_tokens": pricing
                        .input_micros_per_million_tokens(),
                    "output_micros_per_million_tokens": pricing
                        .output_micros_per_million_tokens(),
                })),
            "active": config.active_provider() == Some(name),
        }),
        format!("Provider {name} configured"),
    ))
}

fn parse_pricing(value: &str, name: &str) -> Result<u64, CliError> {
    value.parse::<u64>().map_err(|_| {
        CliError::usage(format!(
            "--{name} must be a non-negative integer in micro-units per million tokens"
        ))
    })
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
    let model = parsed
        .value("model")
        .or(config.provider_model())
        .unwrap_or("default");
    let provider = configured_provider(&config, model, "provider test")?;
    let manifest = provider.manifest().clone();
    let request = ModelRequest::new(
        manifest.id().clone(),
        manifest.default_model().clone(),
        vec![
            ChatMessage::user("Reply with exactly: ready")
                .map_err(|error| CliError::provider(error.to_string(), json!({})))?,
        ],
    )
    .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
    let workspace = WorkspaceRoot::new(config.workspace_dir())
        .map_err(|_| CliError::configuration("workspace path is invalid", json!({})))?;
    let controller = ExecutionController::with_policy(
        workspace,
        PolicyContext::new(1, [Capability::ProviderInvoke], []),
    );
    let (principal, tenant, workspace_id) = super::session_scope();
    let session = Session::new(
        SessionId::new("provider-test").expect("provider test session ID is valid"),
        principal,
        tenant,
        workspace_id,
        timestamp(),
    );
    let response = controller
        .invoke_provider(provider.as_ref(), request, &session, timestamp())
        .map_err(|_| CliError::provider("provider effect was not authorized", json!({})))?
        .into_result()
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
