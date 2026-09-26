use super::{load_config, parse_options, require_config_file};
use crate::output::{CliError, CommandResult, success};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

const MAX_RECORD_BYTES: u64 = 4 * 1024 * 1024;
const MAX_CRASH_REPORTS: usize = 20;

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("operations requires 'telemetry' or 'crashes'"))?;
    match subcommand.as_str() {
        "telemetry" => telemetry(&args[1..]),
        "crashes" => crashes(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown operations command '{unknown}'"
        ))),
    }
}

fn telemetry(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "operations telemetry does not accept positional arguments",
        ));
    }
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let path = config.data_dir().join("operations/telemetry.jsonl");
    let records = read_jsonl(&path)?;
    let count = records.len();
    Ok(success(
        "operations telemetry",
        json!({"path": path, "count": count, "records": records}),
        format!("Read {count} operational telemetry record(s)"),
    ))
}

fn crashes(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "operations crashes does not accept positional arguments",
        ));
    }
    let config = load_config(&parsed)?;
    require_config_file(&config)?;
    let directory = config.data_dir().join("operations/crashes");
    let mut paths = if directory.is_dir() {
        fs::read_dir(&directory)
            .map_err(|error| read_error(&directory, error))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("crash-") && name.ends_with(".json"))
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    paths.sort();
    let truncated = paths.len() > MAX_CRASH_REPORTS;
    if truncated {
        paths = paths.split_off(paths.len() - MAX_CRASH_REPORTS);
    }
    let mut reports = Vec::with_capacity(paths.len());
    for path in &paths {
        reports.push(read_json(path)?);
    }
    let count = reports.len();
    Ok(success(
        "operations crashes",
        json!({"directory": directory, "count": count, "truncated": truncated, "reports": reports}),
        format!("Read {count} crash report(s)"),
    ))
}

fn read_jsonl(path: &Path) -> Result<Vec<Value>, CliError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let metadata = fs::metadata(path).map_err(|error| read_error(path, error))?;
    if metadata.len() > MAX_RECORD_BYTES {
        return Err(CliError::internal(
            "operations telemetry exceeds its read bound",
            json!({"path": path}),
        ));
    }
    let text = fs::read_to_string(path).map_err(|error| read_error(path, error))?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(|error| read_error(path, error)))
        .collect()
}

fn read_json(path: &Path) -> Result<Value, CliError> {
    let metadata = fs::metadata(path).map_err(|error| read_error(path, error))?;
    if metadata.len() > MAX_RECORD_BYTES {
        return Err(CliError::internal(
            "crash report exceeds its read bound",
            json!({"path": path}),
        ));
    }
    let text = fs::read_to_string(path).map_err(|error| read_error(path, error))?;
    serde_json::from_str(&text).map_err(|error| read_error(path, error))
}

fn read_error(path: &Path, error: impl std::fmt::Display) -> CliError {
    CliError::internal(
        "could not read operations evidence",
        json!({"path": path, "error": error.to_string()}),
    )
}
