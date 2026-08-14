use super::{load_config, parse_options, write_config};
use crate::output::{CliError, CommandResult, success};
use pandora_provider::{ChatMessage, HttpProvider, ModelRequest, Provider, ProviderManifest};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("provider requires 'list', 'set', or 'test'"))?;
    match subcommand.as_str() {
        "list" => list(&args[1..]),
        "set" => set(&args[1..]),
        "test" => test(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown provider command '{unknown}'"
        ))),
    }
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let config = load_config(&parsed)?;
    let providers = match config.provider_url() {
        Some(base_url) => {
            let manifest = ProviderManifest::new(
                "openai-compatible",
                "OpenAI-compatible",
                base_url,
                "default",
                "PANDORA_PROVIDER_API_KEY",
            )
            .map_err(|error| CliError::provider(error.to_string(), json!({})))?;
            let manifest = serde_json::to_value(manifest).map_err(|_| {
                CliError::internal("could not serialize provider metadata", json!({}))
            })?;
            vec![manifest]
        }
        None => Vec::new(),
    };
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
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "provider-url"])?;
    if parsed.value("provider-url").is_none() {
        return Err(CliError::usage(
            "provider set requires '--provider-url <url>'",
        ));
    }
    let config = load_config(&parsed)?;
    write_config(&config)?;
    Ok(success(
        "provider set",
        json!({
            "provider": "openai-compatible",
            "base_url": config.provider_url(),
        }),
        "OpenAI-compatible provider configured".to_owned(),
    ))
}

fn test(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "model"])?;
    let config = load_config(&parsed)?;
    super::require_config_file(&config)?;
    let base_url = config.provider_url().ok_or_else(|| {
        CliError::configuration(
            "provider is not configured; run 'pandora provider set' first",
            json!({"config_path": config.config_path()}),
        )
    })?;
    let model = parsed.value("model").unwrap_or("default");
    let manifest = ProviderManifest::new(
        "openai-compatible",
        "OpenAI-compatible",
        base_url,
        model,
        "PANDORA_PROVIDER_API_KEY",
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
