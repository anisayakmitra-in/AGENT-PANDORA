use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use serde_json::json;
use std::fs;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "dry-run", "yes"])?;
    let dry_run = parsed.value("dry-run").is_some();
    let confirmed = parsed.value("yes").is_some();
    if dry_run && confirmed {
        return Err(CliError::usage(
            "uninstall accepts only one of '--dry-run' or '--yes'",
        ));
    }
    let config = load_config(&parsed)?;
    if !dry_run && !confirmed {
        return Err(CliError::usage(
            "uninstall requires '--yes' or '--dry-run'; workspace files are always preserved",
        ));
    }
    validate_workspace_boundary(config.data_dir(), config.workspace_dir())?;
    let config_path = config.config_path().to_path_buf();
    let data_dir = config.data_dir().to_path_buf();
    if dry_run {
        return Ok(success(
            "uninstall",
            json!({
                "dry_run": true,
                "would_remove": [config_path, data_dir],
                "preserved": [config.workspace_dir()],
            }),
            "Uninstall preview complete; user data was not changed".to_owned(),
        ));
    }
    if !config_path.is_file() {
        return Err(CliError::configuration(
            "Pandora configuration file is missing; nothing was removed",
            json!({"config_path": config_path}),
        ));
    }
    let config_is_in_data_dir = data_dir_contains_path(&data_dir, &config_path);
    remove_data_dir(&data_dir)?;
    if !config_is_in_data_dir {
        fs::remove_file(&config_path).map_err(|_| {
            CliError::configuration(
                "could not remove Pandora configuration",
                json!({"config_path": config_path}),
            )
        })?;
    }
    Ok(success(
        "uninstall",
        json!({
            "dry_run": false,
            "removed": [config_path, data_dir],
            "preserved": [config.workspace_dir()],
        }),
        "Pandora configuration and data removed; workspace preserved".to_owned(),
    ))
}

fn validate_workspace_boundary(
    data_dir: &std::path::Path,
    workspace: &std::path::Path,
) -> Result<(), CliError> {
    if data_dir == workspace {
        return Err(CliError::configuration(
            "data and workspace directories must be different before uninstall",
            json!({"data_dir": data_dir, "workspace": workspace}),
        ));
    }
    if !data_dir.exists() || !workspace.exists() {
        return Ok(());
    }
    let data_dir = fs::canonicalize(data_dir).map_err(|_| {
        CliError::configuration(
            "could not resolve the Pandora data directory",
            json!({"data_dir": data_dir}),
        )
    })?;
    let workspace = fs::canonicalize(workspace).map_err(|_| {
        CliError::configuration(
            "could not resolve the workspace directory",
            json!({"workspace": workspace}),
        )
    })?;
    if workspace.starts_with(&data_dir) {
        return Err(CliError::configuration(
            "Pandora data directory contains the workspace and cannot be removed",
            json!({"data_dir": data_dir, "workspace": workspace}),
        ));
    }
    Ok(())
}

fn data_dir_contains_path(data_dir: &std::path::Path, path: &std::path::Path) -> bool {
    if path.starts_with(data_dir) {
        return true;
    }
    match (fs::canonicalize(data_dir), fs::canonicalize(path)) {
        (Ok(data_dir), Ok(path)) => path.starts_with(data_dir),
        _ => false,
    }
}

fn remove_data_dir(path: &std::path::Path) -> Result<(), CliError> {
    if !path.exists() {
        return Ok(());
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        CliError::configuration(
            "could not inspect Pandora data directory",
            json!({"path": path}),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(CliError::configuration(
            "Pandora data path is not a regular directory",
            json!({"path": path}),
        ));
    }
    fs::remove_dir_all(path).map_err(|_| {
        CliError::configuration(
            "could not remove Pandora data directory",
            json!({"path": path}),
        )
    })
}
