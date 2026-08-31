use super::{load_config, parse_options, timestamp};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{
    MAX_STORAGE_LIFECYCLE_LIST, RecoveryArchive, RecoveryArchiveError, RecoveryEntry,
    StorageLifecycleReceipt, StorageLifecycleStore, StorageLifecycleStoreError,
};
use pandora_types::{StorageLifecycleAction, StorageLifecycleManifest, StorageLifecycleProvider};
use rusqlite::Connection;
use serde_json::json;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use zeroize::Zeroizing;

const MAX_ARCHIVE_FILE_BYTES: u64 = 192 * 1024 * 1024;
const MAX_LIFECYCLE_MANIFEST_BYTES: u64 = 64 * 1024;
const DEFAULT_PASSPHRASE_ENV: &str = "PANDORA_BACKUP_KEY";

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("backup requires 'create', 'inspect', 'restore', or 'lifecycle'")
    })?;
    match subcommand.as_str() {
        "create" => create(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "restore" => restore(&args[1..]),
        "lifecycle" => lifecycle(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown backup command '{unknown}'"
        ))),
    }
}

fn lifecycle(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage("backup lifecycle requires 'preview', 'record', 'list', or 'inspect'")
    })?;
    match subcommand.as_str() {
        "preview" => lifecycle_preview(&args[1..]),
        "record" => lifecycle_record(&args[1..]),
        "list" => lifecycle_list(&args[1..]),
        "inspect" => lifecycle_inspect(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown backup lifecycle command '{unknown}'"
        ))),
    }
}

fn lifecycle_preview(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["input"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "backup lifecycle preview accepts only named options",
        ));
    }
    let input = required_input(&parsed)?;
    let manifest = read_lifecycle_manifest(&input)?;
    Ok(success(
        "backup lifecycle preview",
        json!({
            "dry_run": true,
            "would_record": true,
            "input": input,
            "manifest": lifecycle_manifest_value(&manifest),
            "boundary": lifecycle_boundary(),
        }),
        format!(
            "Validated {} evidence {} for {}; no lifecycle action or evidence write was performed",
            manifest.provider().as_str(),
            manifest.evidence_id(),
            manifest.action().as_str(),
        ),
    ))
}

fn lifecycle_record(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "input", "yes"])?;
    if !parsed.positionals.is_empty() || parsed.value("yes").is_none() {
        return Err(CliError::usage(
            "backup lifecycle record requires '--input <path> --yes'",
        ));
    }
    let input = required_input(&parsed)?;
    let manifest = read_lifecycle_manifest(&input)?;
    let config = load_config(&parsed)?;
    let result = lifecycle_store(&config)?
        .record(manifest, timestamp())
        .map_err(lifecycle_store_error)?;
    let receipt = result.receipt();
    Ok(success(
        "backup lifecycle record",
        json!({
            "created": result.created(),
            "idempotent_replay": !result.created(),
            "receipt": lifecycle_receipt_value(receipt),
            "boundary": lifecycle_boundary(),
        }),
        if result.created() {
            format!(
                "Recorded operator-attested {} evidence {}",
                receipt.manifest().action().as_str(),
                receipt.manifest().evidence_id(),
            )
        } else {
            format!(
                "Evidence {} already matched the append-only ledger; returned the original receipt",
                receipt.manifest().evidence_id(),
            )
        },
    ))
}

fn lifecycle_list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "storage-provider",
            "action",
            "limit",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "backup lifecycle list accepts only named options",
        ));
    }
    let provider = parsed
        .value("storage-provider")
        .map(StorageLifecycleProvider::parse)
        .transpose()
        .map_err(|error| CliError::usage(error.to_string()))?;
    let action = parsed
        .value("action")
        .map(StorageLifecycleAction::parse)
        .transpose()
        .map_err(|error| CliError::usage(error.to_string()))?;
    let limit = parsed
        .value("limit")
        .map(|value| {
            value
                .parse::<usize>()
                .map_err(|_| CliError::usage("backup lifecycle limit must be an integer"))
        })
        .transpose()?
        .unwrap_or(64);
    if limit == 0 || limit > MAX_STORAGE_LIFECYCLE_LIST {
        return Err(CliError::usage(format!(
            "backup lifecycle limit must be between 1 and {MAX_STORAGE_LIFECYCLE_LIST}"
        )));
    }
    let config = load_config(&parsed)?;
    let receipts = lifecycle_store(&config)?
        .list(provider, action, limit)
        .map_err(lifecycle_store_error)?;
    let count = receipts.len();
    Ok(success(
        "backup lifecycle list",
        json!({
            "receipts": receipts.iter().map(lifecycle_receipt_value).collect::<Vec<_>>(),
            "count": count,
            "filters": {
                "provider": provider.map(StorageLifecycleProvider::as_str),
                "action": action.map(StorageLifecycleAction::as_str),
            },
            "boundary": lifecycle_boundary(),
        }),
        format!("Listed {count} append-only storage lifecycle receipt(s)"),
    ))
}

fn lifecycle_inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "id"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "backup lifecycle inspect accepts only named options",
        ));
    }
    let evidence_id = parsed
        .value("id")
        .ok_or_else(|| CliError::usage("backup lifecycle inspect requires '--id <evidence-id>'"))?;
    validate_lifecycle_evidence_id(evidence_id)?;
    let config = load_config(&parsed)?;
    let receipt = lifecycle_store(&config)?
        .inspect(evidence_id)
        .map_err(lifecycle_store_error)?;
    Ok(success(
        "backup lifecycle inspect",
        json!({
            "receipt": lifecycle_receipt_value(&receipt),
            "boundary": lifecycle_boundary(),
        }),
        format!("Inspected storage lifecycle evidence {evidence_id}"),
    ))
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
    reject_unsafe_descendant(
        config.data_dir(),
        &recovery_root.join(".pandora-restore-probe"),
    )?;
    let mut targets = Vec::new();
    for entry in bundle.entries() {
        let target = restore_target(entry.path(), config.config_path(), config.data_dir())?;
        if target == config.config_path() {
            reject_unsafe_target(&target)?;
        } else {
            reject_unsafe_descendant(config.data_dir(), &target)?;
        }
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
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return unsafe_target(path);
    }
    Ok(())
}

fn reject_unsafe_descendant(root: &Path, path: &Path) -> Result<(), CliError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        CliError::configuration("restore target escaped the data directory", json!({}))
    })?;
    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty() {
        return unsafe_target(path);
    }
    let mut current = root.to_path_buf();
    for (index, component) in components.iter().enumerate() {
        match component {
            Component::ParentDir => return unsafe_target(path),
            Component::Prefix(_) | Component::RootDir => return unsafe_target(path),
            Component::CurDir => continue,
            Component::Normal(_) => current.push(component.as_os_str()),
        }
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || (index + 1 < components.len() && !metadata.is_dir())
                    || (index + 1 == components.len() && !metadata.is_file())
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

fn read_lifecycle_manifest(path: &Path) -> Result<StorageLifecycleManifest, CliError> {
    let metadata = fs::symlink_metadata(path).map_err(io_error)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_LIFECYCLE_MANIFEST_BYTES
    {
        return Err(CliError::configuration(
            "storage lifecycle manifest path is unsafe or too large",
            json!({"path": path}),
        ));
    }
    let encoded = fs::read(path).map_err(io_error)?;
    if u64::try_from(encoded.len()).unwrap_or(u64::MAX) > MAX_LIFECYCLE_MANIFEST_BYTES {
        return Err(CliError::configuration(
            "storage lifecycle manifest path is unsafe or too large",
            json!({"path": path}),
        ));
    }
    let manifest: StorageLifecycleManifest = serde_json::from_slice(&encoded).map_err(|error| {
        CliError::configuration(
            format!("storage lifecycle manifest is invalid: {error}"),
            json!({"path": path}),
        )
    })?;
    manifest.validate().map_err(|error| {
        CliError::configuration(
            error.to_string(),
            json!({
                "path": path,
                "external_action_performed_by_runtime": false,
            }),
        )
    })?;
    Ok(manifest)
}

fn lifecycle_store(
    config: &pandora_runtime::config::RuntimeConfig,
) -> Result<StorageLifecycleStore, CliError> {
    StorageLifecycleStore::open(config.data_dir().join("storage-lifecycle.sqlite3"))
        .map_err(lifecycle_store_error)
}

fn lifecycle_manifest_value(manifest: &StorageLifecycleManifest) -> serde_json::Value {
    json!({
        "policy_version": manifest.policy_version(),
        "evidence_id": manifest.evidence_id(),
        "policy_id": manifest.policy_id(),
        "provider": manifest.provider().as_str(),
        "action": manifest.action().as_str(),
        "resource_id": manifest.resource_id(),
        "provider_fields": manifest.provider_fields(),
        "external_evidence_digest": manifest.external_evidence_digest(),
        "actor": manifest.actor(),
        "performed_at": manifest.performed_at(),
        "manifest_digest": manifest.manifest_digest(),
    })
}

fn lifecycle_receipt_value(receipt: &StorageLifecycleReceipt) -> serde_json::Value {
    json!({
        "manifest": lifecycle_manifest_value(receipt.manifest()),
        "manifest_digest": receipt.manifest_digest(),
        "recorded_at": receipt.recorded_at().as_unix_seconds(),
        "evidence_status": receipt.evidence_status(),
        "external_action_performed_by_runtime": receipt.external_action_performed_by_runtime(),
        "secure_erasure_guaranteed": receipt.secure_erasure_guaranteed(),
        "durability": "append-only-sqlite",
    })
}

fn lifecycle_boundary() -> serde_json::Value {
    json!({
        "evidence_status": "operator_attested",
        "external_action_performed_by_runtime": false,
        "secure_erasure_guaranteed": false,
        "runtime_deletes_provider_resources": false,
        "verification_responsibility": "operator",
        "guidance": "Perform and independently verify the provider lifecycle action before recording its digest-bound evidence.",
    })
}

fn validate_lifecycle_evidence_id(value: &str) -> Result<(), CliError> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(byte, b'_' | b'-' | b'.' | b'/' | b':' | b'@' | b'+')
        })
    {
        return Err(CliError::usage("storage lifecycle evidence ID is invalid"));
    }
    Ok(())
}

fn lifecycle_store_error(error: StorageLifecycleStoreError) -> CliError {
    match error {
        StorageLifecycleStoreError::EvidenceConflict => CliError::policy(
            error.to_string(),
            json!({
                "evidence_recorded": false,
                "external_action_performed_by_runtime": false,
            }),
        ),
        StorageLifecycleStoreError::EvidenceNotFound
        | StorageLifecycleStoreError::Contract(_)
        | StorageLifecycleStoreError::PerformedAfterRecord => {
            CliError::configuration(error.to_string(), json!({}))
        }
        _ => CliError::execution(
            error.to_string(),
            json!({
                "evidence_recorded": false,
                "external_action_performed_by_runtime": false,
            }),
        ),
    }
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

        let error =
            reject_unsafe_descendant(&data, &data.join("redirect/state.sqlite3")).unwrap_err();
        assert_eq!(error.code, "configuration_error");
        assert_eq!(error.message, "restore target is unsafe");

        fs::remove_dir_all(root).unwrap();
    }
}
