use super::{load_config, parse_options, write_config};
use crate::output::{CliError, CommandResult, success};
use serde_json::json;
use std::fs;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "provider-url", "model"],
    )?;
    let config = load_config(&parsed)?;
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
        }),
        format!("Pandora configured at {}", config.config_path().display()),
    ))
}
