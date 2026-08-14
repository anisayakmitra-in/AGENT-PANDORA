use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use serde_json::{Value, json};
use std::fs;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let config = load_config(&parsed)?;
    let config_ok = config.config_path().is_file();
    let data_ok = directory_is_readable(config.data_dir());
    let workspace_ok = directory_is_readable(config.workspace_dir());
    let provider_configured = config.provider_url().is_some();
    let checks = vec![
        check(
            "config",
            config_ok,
            "configuration file is available",
            "run 'pandora setup'",
        ),
        check(
            "provider",
            provider_configured,
            "provider configuration is present",
            "run 'pandora provider set --provider-url <url>'",
        ),
        check(
            "storage",
            data_ok,
            "data directory is readable",
            "run 'pandora setup' or check the data directory permissions",
        ),
        check(
            "workspace",
            workspace_ok,
            "workspace directory is readable",
            "check the workspace path and permissions",
        ),
    ];
    let healthy = config_ok && provider_configured && data_ok && workspace_ok;
    let provider = json!({
        "configured": provider_configured,
        "model": config.provider_model().unwrap_or("default"),
        "connectivity": if provider_configured { "not_checked" } else { "not_configured" },
        "remediation": if provider_configured {
            "connectivity checks are opt-in and are not performed by default"
        } else {
            "configure a provider before running model tasks"
        },
    });
    let data = json!({
        "healthy": healthy,
        "version": env!("CARGO_PKG_VERSION"),
        "platform": {
            "os": std::env::consts::OS,
            "architecture": std::env::consts::ARCH,
        },
        "config_path": config.config_path(),
        "storage_path": config.data_dir(),
        "workspace_path": config.workspace_dir(),
        "provider": provider,
        "policy": {
            "mode": "governed",
            "effect_boundary": "reference_monitor",
        },
        "checks": checks,
    });
    if !healthy {
        return Err(CliError::configuration(
            "Pandora diagnostics found configuration issues",
            data,
        ));
    }
    Ok(success(
        "doctor",
        data,
        "Pandora diagnostics passed; provider connectivity was not checked",
    ))
}

fn directory_is_readable(path: &std::path::Path) -> bool {
    path.is_dir() && fs::read_dir(path).is_ok()
}

fn check(name: &str, ok: bool, message: &str, remediation: &str) -> Value {
    json!({
        "check": name,
        "status": if ok { "ok" } else { "failed" },
        "message": message,
        "remediation": if ok { Value::Null } else { Value::String(remediation.to_owned()) },
    })
}
