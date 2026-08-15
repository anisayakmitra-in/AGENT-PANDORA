use super::{load_config, parse_options, write_config};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::config::{
    DEFAULT_PROVIDER_API_KEY_ENV, DEFAULT_PROVIDER_NAME, ProviderProfile,
};
use serde_json::json;
use std::fs;
use std::io::{self, Read, Write};

const MAX_PROMPT_BYTES: usize = 1_024;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "provider-url",
            "model",
            "api-key-env",
            "interactive",
        ],
    )?;
    let mut config = load_config(&parsed)?;
    if parsed.value("interactive").is_some() {
        configure_interactively(&mut config, &parsed)?;
    }
    fs::create_dir_all(config.data_dir()).map_err(|_| {
        CliError::configuration(
            "could not create the Pandora data directory",
            json!({"data_dir": config.data_dir()}),
        )
    })?;
    fs::create_dir_all(config.workspace_dir()).map_err(|_| {
        CliError::configuration(
            "could not create the Pandora workspace directory",
            json!({"workspace": config.workspace_dir()}),
        )
    })?;
    write_config(&config)?;
    let provider_configured = config.provider_url().is_some();
    Ok(success(
        "setup",
        json!({
            "config_path": config.config_path(),
            "data_dir": config.data_dir(),
            "workspace": config.workspace_dir(),
            "provider_configured": provider_configured,
            "provider_profiles": config.provider_names(),
            "active_provider": config.active_provider(),
            "provider_model": config.provider_model().unwrap_or("default"),
            "api_key_env": config.provider_api_key_env(),
            "interactive": parsed.value("interactive").is_some(),
        }),
        format!("Pandora configured at {}", config.config_path().display()),
    ))
}

fn configure_interactively(
    config: &mut pandora_runtime::config::RuntimeConfig,
    parsed: &super::ParsedArgs,
) -> Result<(), CliError> {
    let provider_url = prompt(
        "Provider URL (leave blank for local-only setup)",
        parsed
            .value("provider-url")
            .or_else(|| config.provider_url()),
    )?;
    if provider_url.is_empty() {
        return Ok(());
    }
    let model = prompt(
        "Default model",
        parsed
            .value("model")
            .or_else(|| config.provider_model())
            .or(Some("default")),
    )?;
    let api_key_env = prompt(
        "API key environment variable",
        parsed
            .value("api-key-env")
            .or_else(|| config.provider_api_key_env())
            .or(Some(DEFAULT_PROVIDER_API_KEY_ENV)),
    )?;
    let profile = ProviderProfile::new(DEFAULT_PROVIDER_NAME, provider_url, model, api_key_env)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    config.set_provider_profile(profile);
    config
        .set_active_provider(DEFAULT_PROVIDER_NAME)
        .map_err(|error| CliError::configuration(error.to_string(), json!({})))
}

fn prompt(label: &str, default: Option<&str>) -> Result<String, CliError> {
    eprint!("{label}");
    if let Some(default) = default.filter(|value| !value.is_empty()) {
        eprint!(" [{default}]");
    }
    eprint!(": ");
    io::stderr()
        .flush()
        .map_err(|_| CliError::internal("could not display setup prompt", json!({})))?;

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1];
    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|_| CliError::configuration("could not read setup input", json!({})))?;
        if count == 0 || buffer[0] == b'\n' {
            break;
        }
        if buffer[0] != b'\r' {
            bytes.push(buffer[0]);
        }
        if bytes.len() > MAX_PROMPT_BYTES {
            return Err(CliError::usage("setup input exceeds 1024 bytes"));
        }
    }
    let value =
        String::from_utf8(bytes).map_err(|_| CliError::usage("setup input must be valid UTF-8"))?;
    if value.trim().is_empty() {
        Ok(default.unwrap_or_default().to_owned())
    } else {
        Ok(value.trim().to_owned())
    }
}
