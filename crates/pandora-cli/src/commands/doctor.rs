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
    let credential_available = config
        .provider_api_key_env()
        .is_some_and(credential_is_available);
    let mut checks = vec![
        check(
            "config",
            config_ok,
            "configuration file is available",
            "run 'pandora setup'",
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
    checks.insert(
        1,
        if provider_configured {
            check(
                "provider",
                true,
                "provider configuration is present",
                "run 'pandora provider set --provider-url <url>'",
            )
        } else {
            json!({
                "check": "provider",
                "status": "not_configured",
                "message": "local-only mode is available for read-only tasks",
                "remediation": "configure a provider before running model-backed tasks",
            })
        },
    );
    if provider_configured {
        checks.insert(
            2,
            check(
                "credential",
                credential_available,
                "provider credential environment is available",
                "set the configured provider credential environment variable",
            ),
        );
    }
    let healthy =
        config_ok && data_ok && workspace_ok && (!provider_configured || credential_available);
    let provider = json!({
        "configured": provider_configured,
        "profiles": config.provider_names(),
        "active_profile": config.active_provider(),
        "model": config.provider_model().unwrap_or("default"),
        "credential_env": config.provider_api_key_env(),
        "credential": if provider_configured {
            if credential_available { "available" } else { "missing" }
        } else {
            "not_configured"
        },
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

fn credential_is_available(name: &str) -> bool {
    std::env::var(name)
        .is_ok_and(|value| !value.trim().is_empty() && !value.chars().any(char::is_control))
}

fn check(name: &str, ok: bool, message: &str, remediation: &str) -> Value {
    json!({
        "check": name,
        "status": if ok { "ok" } else { "failed" },
        "message": message,
        "remediation": if ok { Value::Null } else { Value::String(remediation.to_owned()) },
    })
}
