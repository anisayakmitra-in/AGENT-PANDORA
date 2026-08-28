use super::{load_config, parse_options, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{RecoveryArchive, RecoveryArchiveError, RecoveryEntry};
use rusqlite::Connection;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use zeroize::Zeroizing;

const MAX_ARCHIVE_FILE_BYTES: u64 = 192 * 1024 * 1024;
const DEFAULT_PASSPHRASE_ENV: &str = "PANDORA_BACKUP_KEY";

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("backup requires 'create', 'inspect', or 'restore'"))?;
    match subcommand.as_str() {
        "create" => create(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "restore" => restore(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown backup command '{unknown}'"
        ))),
    }
}

fn create(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "output",
            "passphrase-env",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage("backup create accepts only named options"));
    }
    let output = PathBuf::from(
        parsed
            .value("output")
            .ok_or_else(|| CliError::usage("backup create requires '--output <path>'"))?,
    );
    let config = load_config(&parsed)?;
    let passphrase = backup_passphrase(&parsed)?;
    let entries = collect_entries(config.config_path(), config.data_dir(), &output)?;
    if entries.is_empty() {
        return Err(CliError::configuration(
            "Pandora has no configured state to back up",
            json!({}),
        ));
    }
    let entry_count = entries.len();
    let archive = RecoveryArchive::seal(entries, &passphrase, timestamp().as_unix_seconds())
        .map_err(recovery_error)?;
    write_private_atomic(&output, &archive)?;
    Ok(success(
        "backup create",
        json!({
            "output": output,
            "entries": entry_count,
            "encrypted": true,
            "format_version": 1,
        }),
        format!(
            "Encrypted recovery archive created at {} with {entry_count} entries",
            output.display()
        ),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "input", "passphrase-env"],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage("backup inspect accepts only named options"));
    }
    let input = required_input(&parsed)?;
    let passphrase = backup_passphrase(&parsed)?;
    let encoded = read_archive(&input)?;
    let bundle = RecoveryArchive::open(&encoded, &passphrase).map_err(recovery_error)?;
    Ok(success(
        "backup inspect",
        json!({
            "input": input,
            "created_at": bundle.created_at(),
            "entries": bundle.entries().len(),
            "authenticated": true,
            "paths_exposed": false,
        }),
        format!(
            "Recovery archive authenticated with {} entries",
            bundle.entries().len()
        ),
    ))
}

fn restore(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "input",
            "passphrase-env",
            "yes",
        ],
    )?;
    if !parsed.positionals.is_empty() || parsed.value("yes").is_none() {
        return Err(CliError::usage(
            "backup restore requires '--input <path> --yes'",
        ));
    }
    let input = required_input(&parsed)?;
    let config = load_config(&parsed)?;
    let passphrase = backup_passphrase(&parsed)?;
    let encoded = read_archive(&input)?;
    let bundle = RecoveryArchive::open(&encoded, &passphrase).map_err(recovery_error)?;
    validate_sqlite_entries(bundle.entries())?;
    let recovery_root = config
        .data_dir()
        .join("recovery")
        .join(format!("pre-restore-{}", timestamp().as_unix_seconds()));
    reject_unsafe_target(&recovery_root)?;
    let mut targets = Vec::new();
    for entry in bundle.entries() {
        let target = restore_target(entry.path(), config.config_path(), config.data_dir())?;
        reject_unsafe_target(&target)?;
        targets.push((entry, target));
    }
    let mut originals = BTreeMap::new();
    for (_, target) in &targets {
        if target.is_file() {
            let backup = recovery_root.join(backup_relative_path(
                target,
                config.config_path(),
                config.data_dir(),
            )?);
            if let Some(parent) = backup.parent() {
                fs::create_dir_all(parent).map_err(io_error)?;
            }
            fs::copy(target, &backup).map_err(io_error)?;
            originals.insert(target.clone(), Some(backup));
        } else {
            originals.insert(target.clone(), None);
        }
    }
    let mut written = Vec::new();
    for (entry, target) in &targets {
        if let Err(error) = write_private_atomic(target, entry.bytes()) {
            rollback_restore(&written, &originals);
            return Err(error);
        }
        written.push(target.clone());
    }
    Ok(success(
        "backup restore",
        json!({
            "input": input,
            "restored_entries": written.len(),
            "pre_restore_backup": recovery_root,
            "authenticated": true,
        }),
        format!(
            "Restored {} Pandora state entries; previous state is at {}",
            written.len(),
            recovery_root.display()
        ),
    ))
}

fn collect_entries(
    config_path: &Path,
    data_dir: &Path,
    output: &Path,
) -> Result<Vec<RecoveryEntry>, CliError> {
    let mut files = Vec::new();
    if config_path.is_file() {
        files.push(("config/config.json".to_owned(), config_path.to_path_buf()));
    }
    if data_dir.is_dir() {
        collect_data_files(data_dir, data_dir, output, &mut files)?;
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
        .into_iter()
        .map(|(archive_path, source)| {
            let bytes = fs::read(&source).map_err(io_error)?;
            RecoveryEntry::new(archive_path, bytes).map_err(recovery_error)
        })
        .collect()
}

fn collect_data_files(
    root: &Path,
    directory: &Path,
    output: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), CliError> {
    let mut entries = fs::read_dir(directory)
        .map_err(io_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io_error)?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let path = entry.path();
        if path == output {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        if metadata.file_type().is_symlink() {
            return Err(CliError::configuration(
                "backup source contains a symbolic link",
                json!({"path": path}),
            ));
        }
        let relative = path.strip_prefix(root).map_err(|_| {
            CliError::configuration("backup source escaped the data directory", json!({}))
        })?;
        if relative.components().next().is_some_and(|component| {
            matches!(component, Component::Normal(name) if matches!(name.to_str(), Some("operations" | "updates" | "recovery")))
        }) {
            continue;
        }
        if metadata.is_dir() {
            collect_data_files(root, &path, output, files)?;
        } else if metadata.is_file() {
            files.push((
                format!("data/{}", relative.to_string_lossy().replace('\\', "/")),
                path,
            ));
        }
    }
    Ok(())
}

fn validate_sqlite_entries(entries: &[RecoveryEntry]) -> Result<(), CliError> {
    let staging = std::env::temp_dir().join(format!(
        "pandora-restore-validation-{}-{}",
        std::process::id(),
        timestamp().as_unix_seconds()
    ));
    fs::create_dir_all(&staging).map_err(io_error)?;
    let result = entries
        .iter()
        .filter(|entry| entry.path().ends_with(".sqlite3"))
        .enumerate()
        .try_for_each(|(index, entry)| {
            let path = staging.join(format!("{index}.sqlite3"));
            fs::write(&path, entry.bytes()).map_err(io_error)?;
            let connection = Connection::open(&path).map_err(|_| {
                CliError::configuration("backup contains an invalid SQLite database", json!({}))
            })?;
            let status: String = connection
                .query_row("PRAGMA quick_check", [], |row| row.get(0))
                .map_err(|_| {
                    CliError::configuration(
                        "backup SQLite integrity check could not run",
                        json!({}),
                    )
                })?;
            if status != "ok" {
                return Err(CliError::configuration(
                    "backup SQLite integrity check failed",
                    json!({}),
                ));
            }
            Ok(())
        });
    let _ = fs::remove_dir_all(staging);
    result
}

fn restore_target(
    archive_path: &str,
    config_path: &Path,
    data_dir: &Path,
) -> Result<PathBuf, CliError> {
    if archive_path == "config/config.json" {
        return Ok(config_path.to_path_buf());
    }
    let relative = archive_path.strip_prefix("data/").ok_or_else(|| {
        CliError::configuration("backup contains an unsupported restore target", json!({}))
    })?;
    if relative.is_empty() {
        return Err(CliError::configuration(
            "backup contains an unsupported restore target",
            json!({}),
        ));
    }
    Ok(data_dir.join(relative))
}

fn backup_relative_path(
    target: &Path,
    config_path: &Path,
    data_dir: &Path,
) -> Result<PathBuf, CliError> {
    if target == config_path {
        return Ok(PathBuf::from("config/config.json"));
    }
    target
        .strip_prefix(data_dir)
        .map(|relative| Path::new("data").join(relative))
        .map_err(|_| {
            CliError::configuration("restore target escaped the data directory", json!({}))
        })
}

fn reject_unsafe_target(path: &Path) -> Result<(), CliError> {
    let components = path.components().collect::<Vec<_>>();
    let mut current = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::ParentDir => return unsafe_target(path),
            Component::Prefix(_) | Component::RootDir | Component::CurDir => {
                current.push(component.as_os_str());
                continue;
            }
            Component::Normal(_) => current.push(component.as_os_str()),
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || (index + 1 < components.len() && !metadata.is_dir())
                    || (index + 1 == components.len()
                        && !metadata.is_file()
                        && path.extension().is_some())
                {
                    return unsafe_target(path);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return unsafe_target(path),
        }
    }
    Ok(())
}

fn unsafe_target(path: &Path) -> Result<(), CliError> {
    Err(CliError::configuration(
        "restore target is unsafe",
        json!({"path": path}),
    ))
}

fn rollback_restore(written: &[PathBuf], originals: &BTreeMap<PathBuf, Option<PathBuf>>) {
    for target in written.iter().rev() {
        match originals.get(target) {
            Some(Some(backup)) => {
                let _ = fs::copy(backup, target);
            }
            Some(None) => {
                let _ = fs::remove_file(target);
            }
            None => {}
        }
    }
}

fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), CliError> {
    reject_unsafe_target(path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error)?;
    }
    let mut file = atomic_write_file::AtomicWriteFile::open(path).map_err(io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(io_error)?;
    }
    file.write_all(bytes).map_err(io_error)?;
    file.commit().map_err(io_error)
}

fn backup_passphrase(parsed: &super::ParsedArgs) -> Result<Zeroizing<String>, CliError> {
    let name = parsed
        .value("passphrase-env")
        .unwrap_or(DEFAULT_PASSPHRASE_ENV);
    if name.is_empty()
        || !name.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_uppercase() || (index > 0 && byte.is_ascii_digit())
        })
    {
        return Err(CliError::usage("--passphrase-env is invalid"));
    }
    std::env::var(name).map(Zeroizing::new).map_err(|_| {
        CliError::configuration(
            format!("recovery passphrase environment variable '{name}' is not set"),
            json!({"environment": name}),
        )
    })
}

fn required_input(parsed: &super::ParsedArgs) -> Result<PathBuf, CliError> {
    parsed
        .value("input")
        .map(PathBuf::from)
        .ok_or_else(|| CliError::usage("backup command requires '--input <path>'"))
}

fn read_archive(path: &Path) -> Result<Vec<u8>, CliError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_ARCHIVE_FILE_BYTES
    {
        return Err(CliError::configuration(
            "recovery archive path is unsafe or too large",
            json!({"path": path}),
        ));
    }
    fs::read(path).map_err(io_error)
}

fn recovery_error(error: RecoveryArchiveError) -> CliError {
    CliError::configuration(error.to_string(), json!({}))
}

fn io_error(error: std::io::Error) -> CliError {
    CliError::configuration(error.to_string(), json!({}))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn restore_rejects_a_symlink_in_any_parent_component() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "pandora-restore-path-test-{}-{suffix}",
            std::process::id()
        ));
        let data = root.join("data");
        let outside = root.join("outside");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, data.join("redirect")).unwrap();

        let error = reject_unsafe_target(&data.join("redirect/state.sqlite3")).unwrap_err();
        assert_eq!(error.code, "configuration_error");
        assert_eq!(error.message, "restore target is unsafe");

        fs::remove_dir_all(root).unwrap();
    }
}
