use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use serde_json::json;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let config = load_config(&parsed)?;
    let mut failures = Vec::new();
    if !config.config_path().is_file() {
        failures.push(json!({
            "check": "config",
            "status": "failed",
            "message": "configuration file is missing"
        }));
    }
    if config.provider_url().is_none() {
        failures.push(json!({
            "check": "provider",
            "status": "failed",
            "message": "no provider is configured"
        }));
    }
    if !config.data_dir().is_dir() {
        failures.push(json!({
            "check": "data_dir",
            "status": "failed",
            "message": "data directory is missing"
        }));
    }
    if !config.workspace_dir().is_dir() {
        failures.push(json!({
            "check": "workspace",
            "status": "failed",
            "message": "workspace directory is missing"
        }));
    }
    if !failures.is_empty() {
        return Err(CliError::configuration(
            "Pandora configuration checks failed",
            json!({"checks": failures}),
        ));
    }
    let checks = vec![
        json!({"check": "config", "status": "ok"}),
        json!({"check": "provider", "status": "ok"}),
        json!({"check": "data_dir", "status": "ok"}),
        json!({"check": "workspace", "status": "ok"}),
    ];
    Ok(success(
        "doctor",
        json!({"healthy": true, "checks": checks}),
        "Pandora configuration is healthy".to_owned(),
    ))
}
