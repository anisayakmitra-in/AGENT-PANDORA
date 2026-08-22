use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_runtime::{MAX_STORED_ARTIFACT_BYTES, PackageRecord, PackageStore, PackageStoreError};
use pandora_types::{PackageId, PackageManifest};
use serde_json::json;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "package requires 'admit', 'list', 'inspect', 'lock', 'verify-lock', or 'remove'",
        )
    })?;
    match subcommand.as_str() {
        "admit" => admit(&args[1..]),
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "lock" => lock(&args[1..]),
        "verify-lock" => verify_lock(&args[1..]),
        "remove" => remove(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown package command '{unknown}'"
        ))),
    }
}

fn lock(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "output"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package lock does not accept positional arguments",
        ));
    }
    let config = load_config(&parsed)?;
    let path = parsed
        .value("output")
        .map(PathBuf::from)
        .unwrap_or_else(|| config.workspace_dir().join("pandora.lock"));
    let store =
        PackageStore::open(config.data_dir().join("packages.sqlite3")).map_err(store_error)?;
    let lock = store.write_lockfile(&path).map_err(store_error)?;
    Ok(success(
        "package lock",
        json!({
            "path": path,
            "format_version": lock.format_version(),
            "package_count": lock.packages().len(),
        }),
        format!(
            "Locked {} package(s) in {}",
            lock.packages().len(),
            path.display()
        ),
    ))
}

fn verify_lock(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "lock"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package verify-lock does not accept positional arguments",
        ));
    }
    let config = load_config(&parsed)?;
    let path = parsed
        .value("lock")
        .map(PathBuf::from)
        .unwrap_or_else(|| config.workspace_dir().join("pandora.lock"));
    let store =
        PackageStore::open(config.data_dir().join("packages.sqlite3")).map_err(store_error)?;
    let lock = store.verify_lockfile(&path).map_err(store_error)?;
    Ok(success(
        "package verify-lock",
        json!({
            "verified": true,
            "path": path,
            "format_version": lock.format_version(),
            "package_count": lock.packages().len(),
        }),
        format!("Package lock verified: {}", path.display()),
    ))
}

fn admit(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &["config", "data-dir", "workspace", "manifest", "artifact"],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package admit does not accept positional arguments",
        ));
    }
    let manifest_path = required_path(&parsed, "manifest")?;
    let artifact_path = required_path(&parsed, "artifact")?;
    let manifest_bytes = fs::read(&manifest_path).map_err(|error| {
        CliError::configuration(
            "could not read package manifest",
            json!({"path": manifest_path, "error": error.to_string()}),
        )
    })?;
    let manifest: PackageManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| CliError::usage(format!("package manifest is invalid: {error}")))?;
    let artifact = read_artifact(&artifact_path)?;
    let store = store(&parsed)?;
    let record = store
        .admit(&manifest, &manifest, &artifact)
        .map_err(store_error)?;
    let id = record.manifest().id().as_str().to_owned();
    Ok(success(
        "package admit",
        json!({"package": package_value(&record)}),
        format!("Package {id}@{} admitted", record.manifest().version()),
    ))
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package list does not accept positional arguments",
        ));
    }
    let records = store(&parsed)?.list().map_err(store_error)?;
    let count = records.len();
    Ok(success(
        "package list",
        json!({
            "packages": records.iter().map(package_value).collect::<Vec<_>>()
        }),
        format!("{count} package(s) admitted locally"),
    ))
}

fn inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if parsed.positionals.len() != 2 {
        return Err(CliError::usage(
            "package inspect requires an ID and an exact version",
        ));
    }
    let id = PackageId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("package ID is invalid"))?;
    let version = &parsed.positionals[1];
    let record = store(&parsed)?
        .get(&id, version)
        .map_err(store_error)?
        .ok_or_else(|| {
            CliError::execution(
                "package was not admitted locally",
                json!({"id": id.as_str(), "version": version}),
            )
        })?;
    Ok(success(
        "package inspect",
        json!({"package": package_value(&record)}),
        format!("{}@{}", id.as_str(), version),
    ))
}

fn remove(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "dry-run", "yes"])?;
    if parsed.positionals.len() != 2 {
        return Err(CliError::usage(
            "package remove requires an ID and an exact version",
        ));
    }
    let dry_run = parsed.value("dry-run").is_some();
    let confirmed = parsed.value("yes").is_some();
    if dry_run && confirmed {
        return Err(CliError::usage(
            "package remove accepts only one of '--dry-run' or '--yes'",
        ));
    }
    if !dry_run && !confirmed {
        return Err(CliError::usage(
            "package remove requires '--yes' or '--dry-run'",
        ));
    }

    let id = PackageId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("package ID is invalid"))?;
    let version = &parsed.positionals[1];
    let store = store(&parsed)?;
    let record = store
        .get(&id, version)
        .map_err(store_error)?
        .ok_or_else(|| {
            CliError::execution(
                "package was not admitted locally",
                json!({"id": id.as_str(), "version": version}),
            )
        })?;
    if dry_run {
        return Ok(success(
            "package remove",
            json!({
                "dry_run": true,
                "removed": false,
                "package": package_value(&record),
            }),
            format!(
                "Package {}@{} is admitted; no files changed",
                id.as_str(),
                version
            ),
        ));
    }

    let removed = store
        .remove(&id, version)
        .map_err(removal_error)?
        .ok_or_else(|| {
            CliError::execution(
                "package was not admitted locally",
                json!({"id": id.as_str(), "version": version}),
            )
        })?;
    Ok(success(
        "package remove",
        json!({
            "dry_run": false,
            "removed": true,
            "package": package_value(&removed),
        }),
        format!("Package {}@{} removed", id.as_str(), version),
    ))
}

fn required_path(parsed: &super::ParsedArgs, name: &str) -> Result<PathBuf, CliError> {
    parsed
        .value(name)
        .map(PathBuf::from)
        .ok_or_else(|| CliError::usage(format!("package admit requires '--{name} <path>'")))
}

fn read_artifact(path: &Path) -> Result<Vec<u8>, CliError> {
    let mut file = fs::File::open(path).map_err(|error| artifact_read_error(path, error))?;
    match read_artifact_bytes(&mut file, MAX_STORED_ARTIFACT_BYTES) {
        Ok(artifact) => Ok(artifact),
        Err(ArtifactReadError::TooLarge) => Err(CliError::execution(
            "package artifact exceeds the local limit",
            json!({"path": path, "limit_bytes": MAX_STORED_ARTIFACT_BYTES}),
        )),
        Err(ArtifactReadError::Io(error)) => Err(artifact_read_error(path, error)),
    }
}

fn artifact_read_error(path: &Path, error: io::Error) -> CliError {
    CliError::configuration(
        "could not read package artifact",
        json!({"path": path, "error": error.to_string()}),
    )
}

#[derive(Debug)]
enum ArtifactReadError {
    Io(io::Error),
    TooLarge,
}

fn read_artifact_bytes<R: Read>(
    reader: &mut R,
    limit: usize,
) -> Result<Vec<u8>, ArtifactReadError> {
    let mut artifact = Vec::new();
    reader
        .take(limit as u64 + 1)
        .read_to_end(&mut artifact)
        .map_err(ArtifactReadError::Io)?;
    if artifact.len() > limit {
        return Err(ArtifactReadError::TooLarge);
    }
    Ok(artifact)
}

pub(super) fn store(parsed: &super::ParsedArgs) -> Result<PackageStore, CliError> {
    let config = load_config(parsed)?;
    PackageStore::open(config.data_dir().join("packages.sqlite3")).map_err(store_error)
}

pub(super) fn package_value(record: &PackageRecord) -> serde_json::Value {
    let manifest = record.manifest();
    json!({
        "id": manifest.id().as_str(),
        "version": manifest.version(),
        "kind": manifest.kind().as_str(),
        "publisher": manifest.publisher(),
        "content_hash": manifest.content_hash(),
        "dependencies": manifest.dependencies().iter().map(|dependency| json!({
            "id": dependency.id().as_str(),
            "version": dependency.version(),
            "optional": dependency.optional(),
        })).collect::<Vec<_>>(),
        "compatibility": manifest.compatibility().runtime(),
        "license": manifest.license(),
        "trust": {
            "level": manifest.trust().level(),
            "has_signature": manifest.trust().signature().is_some(),
            "has_public_key": manifest.trust().public_key().is_some(),
        },
        "meta_composition": manifest.meta_composition().map(|composition| json!({
            "allowed_domains": composition.allowed_domains().iter().map(|id| id.as_str()).collect::<Vec<_>>(),
            "max_handoffs": composition.max_handoffs(),
        })),
        "state": record.state().as_str(),
        "runtime_authority": record.grants_runtime_authority(),
    })
}

pub(super) fn store_error(error: PackageStoreError) -> CliError {
    CliError::execution(error.to_string(), json!({}))
}

fn removal_error(error: PackageStoreError) -> CliError {
    match error {
        PackageStoreError::HasDependents {
            id,
            version,
            dependents,
        } => CliError::policy(
            format!("cannot remove package {id}@{version} because required dependents exist"),
            json!({"id": id, "version": version, "dependents": dependents}),
        ),
        other => store_error(other),
    }
}

#[cfg(test)]
mod tests {
    use super::{ArtifactReadError, read_artifact_bytes};
    use std::io::Cursor;

    #[test]
    fn artifact_reader_stops_after_the_limit_plus_one_byte() {
        let mut source = Cursor::new(vec![0_u8; 32]);

        assert!(matches!(
            read_artifact_bytes(&mut source, 8),
            Err(ArtifactReadError::TooLarge)
        ));
        assert_eq!(source.position(), 9);
    }

    #[test]
    fn artifact_reader_accepts_an_exactly_limited_artifact() {
        let mut source = Cursor::new(vec![7_u8; 8]);

        assert_eq!(read_artifact_bytes(&mut source, 8).unwrap(), vec![7_u8; 8]);
    }
}
