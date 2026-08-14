use super::{config_path, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::config::CONFIG_FORMAT_VERSION;
use serde_json::{Map, Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug)]
enum MigrationError {
    Io,
    InvalidJson,
    UnsupportedFormat,
    AlreadyCurrent,
    BackupExists,
}

impl MigrationError {
    fn reason(&self) -> &'static str {
        match self {
            Self::Io => "io_error",
            Self::InvalidJson => "invalid_json",
            Self::UnsupportedFormat => "unsupported_format",
            Self::AlreadyCurrent => "already_current",
            Self::BackupExists => "backup_exists",
        }
    }
}

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("migrate requires 'config'"))?;
    if subcommand != "config" {
        return Err(CliError::usage("migrate supports only 'config'"));
    }
    let parsed = parse_options(&args[1..], &["config", "dry-run"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "migrate config does not accept positional arguments",
        ));
    }
    let path = config_path(&parsed);
    let dry_run = parsed.value("dry-run").is_some();
    migrate_file(&path, dry_run).map_err(|error| {
        CliError::configuration(
            migration_message(&error),
            json!({"reason": error.reason(), "config_path": path}),
        )
    })
}

fn migrate_file(path: &Path, dry_run: bool) -> Result<CommandResult, MigrationError> {
    let bytes = fs::read(path).map_err(|_| MigrationError::Io)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| MigrationError::InvalidJson)?;
    let object = value.as_object().ok_or(MigrationError::InvalidJson)?;
    if object
        .get("format_version")
        .and_then(Value::as_u64)
        .is_some_and(|version| version == u64::from(CONFIG_FORMAT_VERSION))
    {
        return Err(MigrationError::AlreadyCurrent);
    }
    let current = convert(object)?;
    let serialized =
        serde_json::to_vec_pretty(&current).map_err(|_| MigrationError::InvalidJson)?;
    let backup = backup_path(path);
    if !dry_run {
        if backup.exists() {
            return Err(MigrationError::BackupExists);
        }
        atomic_replace(path, &backup, &serialized)?;
    }
    Ok(success(
        "migrate config",
        json!({
            "config_path": path,
            "backup_path": backup,
            "format_version": CONFIG_FORMAT_VERSION,
            "dry_run": dry_run,
        }),
        if dry_run {
            format!("Configuration migration is ready for {}", path.display())
        } else {
            format!(
                "Configuration migrated; backup saved at {}",
                backup.display()
            )
        },
    ))
}

fn convert(object: &Map<String, Value>) -> Result<Map<String, Value>, MigrationError> {
    let mut current = Map::new();
    current.insert(
        "format_version".to_owned(),
        Value::from(u64::from(CONFIG_FORMAT_VERSION)),
    );

    let provider_url = object
        .get("provider_url")
        .cloned()
        .or_else(|| object.get("provider").and_then(legacy_provider_url));
    let provider_model = object.get("provider_model").cloned();
    let data_dir = object
        .get("data_dir")
        .cloned()
        .or_else(|| object.get("data_path").cloned());
    let workspace_dir = object
        .get("workspace_dir")
        .cloned()
        .or_else(|| object.get("workspace_path").cloned());
    if provider_url.is_none()
        && provider_model.is_none()
        && data_dir.is_none()
        && workspace_dir.is_none()
    {
        return Err(MigrationError::UnsupportedFormat);
    }
    if let Some(value) = provider_url {
        if !value.is_string() {
            return Err(MigrationError::UnsupportedFormat);
        }
        current.insert("provider_url".to_owned(), value);
    }
    if let Some(value) = provider_model {
        if !value.is_string() {
            return Err(MigrationError::UnsupportedFormat);
        }
        current.insert("provider_model".to_owned(), value);
    }
    if let Some(value) = data_dir {
        if !value.is_string() {
            return Err(MigrationError::UnsupportedFormat);
        }
        current.insert("data_dir".to_owned(), value);
    }
    if let Some(value) = workspace_dir {
        if !value.is_string() {
            return Err(MigrationError::UnsupportedFormat);
        }
        current.insert("workspace_dir".to_owned(), value);
    }
    Ok(current)
}

fn legacy_provider_url(value: &Value) -> Option<Value> {
    value
        .as_str()
        .map(|url| Value::String(url.to_owned()))
        .or_else(|| {
            value
                .get("url")
                .and_then(Value::as_str)
                .map(|url| Value::String(url.to_owned()))
        })
}

fn backup_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.bak", path.display()))
}

fn atomic_replace(path: &Path, backup: &Path, bytes: &[u8]) -> Result<(), MigrationError> {
    let temporary = PathBuf::from(format!("{}.migrate-{}", path.display(), std::process::id()));
    let write_result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| MigrationError::Io)?;
        file.write_all(bytes).map_err(|_| MigrationError::Io)?;
        file.write_all(b"\n").map_err(|_| MigrationError::Io)?;
        file.sync_all().map_err(|_| MigrationError::Io)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
        return write_result;
    }
    if let Err(error) = fs::rename(path, backup) {
        let _ = fs::remove_file(&temporary);
        return Err(if error.kind() == std::io::ErrorKind::AlreadyExists {
            MigrationError::BackupExists
        } else {
            MigrationError::Io
        });
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::rename(backup, path);
        let _ = fs::remove_file(&temporary);
        return Err(if error.kind() == std::io::ErrorKind::AlreadyExists {
            MigrationError::BackupExists
        } else {
            MigrationError::Io
        });
    }
    Ok(())
}

fn migration_message(error: &MigrationError) -> &'static str {
    match error {
        MigrationError::Io => "configuration migration could not complete",
        MigrationError::InvalidJson => "configuration is not valid JSON",
        MigrationError::UnsupportedFormat => "configuration format is not supported",
        MigrationError::AlreadyCurrent => "configuration is already current",
        MigrationError::BackupExists => {
            "configuration backup already exists; move it before retrying"
        }
    }
}
