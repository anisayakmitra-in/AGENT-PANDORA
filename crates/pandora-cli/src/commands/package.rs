use super::{load_config, parse_options};
use crate::output::{CliError, CommandResult, success};
use pandora_harnesses::{builtin_genes, builtin_harnesses};
use pandora_runtime::config::DEFAULT_REGISTRY_TOKEN_ENV;
use pandora_runtime::{
    ArtifactCatalog, GitHubPackageClient, GitHubPackageError, MAX_STORED_ARTIFACT_BYTES,
    PackageBinding, PackageRecord, PackageRegistryClient, PackageRegistryError, PackageStore,
    PackageStoreError, WasmExecutor,
};
use pandora_types::{ArtifactId, PackageId, PackageKind, PackageManifest, hash_artifact};
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

const DEFAULT_GITHUB_TOKEN_ENV: &str = "PANDORA_GITHUB_TOKEN";

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "package requires 'admit', 'validate', 'install', 'install-github', 'list', 'inspect', 'enable', 'disable', 'rollback', 'lock', 'verify-lock', or 'remove'",
        )
    })?;
    match subcommand.as_str() {
        "admit" => admit(&args[1..]),
        "validate" => validate(&args[1..]),
        "install" => install(&args[1..]),
        "install-github" => install_github(&args[1..]),
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "enable" => enable(&args[1..]),
        "disable" => disable(&args[1..]),
        "rollback" => rollback(&args[1..]),
        "lock" => lock(&args[1..]),
        "verify-lock" => verify_lock(&args[1..]),
        "remove" => remove(&args[1..]),
        unknown => Err(CliError::usage(format!(
            "unknown package command '{unknown}'"
        ))),
    }
}

fn install_github(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "repository",
            "commit",
            "manifest",
            "artifact",
            "token-env",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package install-github does not accept positional arguments",
        ));
    }
    let repository = parsed.value("repository").ok_or_else(|| {
        CliError::usage("package install-github requires '--repository <GitHub URL>'")
    })?;
    let commit = parsed
        .value("commit")
        .ok_or_else(|| CliError::usage("package install-github requires '--commit <full SHA>'"))?;
    let manifest_path = parsed.value("manifest").ok_or_else(|| {
        CliError::usage("package install-github requires '--manifest <repository path>'")
    })?;
    let artifact_path = parsed.value("artifact").ok_or_else(|| {
        CliError::usage("package install-github requires '--artifact <repository path>'")
    })?;
    let token_env = parsed.value("token-env");
    if token_env.is_some_and(str::is_empty) {
        return Err(CliError::usage("--token-env requires a non-empty name"));
    }
    let token_name = token_env.unwrap_or(DEFAULT_GITHUB_TOKEN_ENV);
    let token = match std::env::var(token_name) {
        Ok(token) => Some(token),
        Err(_) if token_env.is_some() => {
            return Err(CliError::configuration(
                "configured GitHub token environment variable is unavailable",
                json!({"token_env": token_name}),
            ));
        }
        Err(_) => None,
    };
    let client = GitHubPackageClient::new(repository, commit, token).map_err(github_error)?;
    let store = store(&parsed)?;
    let record = client
        .install(&store, manifest_path, artifact_path)
        .map_err(github_error)?;
    Ok(success(
        "package install-github",
        json!({
            "source": {
                "kind": "github",
                "repository": repository,
                "commit": commit.to_ascii_lowercase(),
                "manifest_path": manifest_path,
                "artifact_path": artifact_path,
            },
            "package": managed_package_value(&store, &record)?,
        }),
        format!(
            "Package {}@{} admitted from pinned GitHub source",
            record.manifest().id().as_str(),
            record.manifest().version()
        ),
    ))
}

fn install(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "registry",
            "registry-profile",
            "token-env",
        ],
    )?;
    if !(1..=2).contains(&parsed.positionals.len()) {
        return Err(CliError::usage(
            "package install requires an ID and accepts one optional exact version",
        ));
    }
    let id = PackageId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("package ID is invalid"))?;
    let version = parsed.positionals.get(1).map(String::as_str);
    if parsed.value("registry").is_some() && parsed.value("registry-profile").is_some() {
        return Err(CliError::usage(
            "package install accepts either '--registry' or '--registry-profile', not both",
        ));
    }
    let config = load_config(&parsed)?;
    let selected_profile = parsed.value("registry-profile").or_else(|| {
        parsed
            .value("registry")
            .is_none()
            .then(|| config.active_registry())
            .flatten()
    });
    let (registry, profile_token_env, registry_profile) = if let Some(name) = selected_profile {
        let profile = config.registry_profile(name).ok_or_else(|| {
            CliError::configuration(
                "registry profile is not configured",
                json!({"registry_profile": name}),
            )
        })?;
        (
            profile.base_url().to_owned(),
            profile.token_env().map(str::to_owned),
            Some(profile.name().to_owned()),
        )
    } else {
        let registry = parsed
            .value("registry")
            .map(str::to_owned)
            .or_else(|| std::env::var("PANDORA_REGISTRY_URL").ok())
            .ok_or_else(|| {
                CliError::configuration(
                    "package install requires a registry profile, '--registry <url>', or PANDORA_REGISTRY_URL",
                    json!({}),
                )
            })?;
        (registry, None, None)
    };
    let token_env = parsed.value("token-env");
    if token_env.is_some_and(str::is_empty) {
        return Err(CliError::usage("--token-env requires a non-empty name"));
    }
    let token_name = token_env
        .or(profile_token_env.as_deref())
        .unwrap_or(DEFAULT_REGISTRY_TOKEN_ENV);
    let token = super::provider::configured_credential(&config, token_name)?;
    if token.is_none() && (token_env.is_some() || profile_token_env.is_some()) {
        return Err(CliError::configuration(
            "configured registry credential is unavailable",
            json!({"token_env": token_name}),
        ));
    }
    let client = PackageRegistryClient::new(&registry, token).map_err(registry_error)?;
    let store =
        PackageStore::open(config.data_dir().join("packages.sqlite3")).map_err(store_error)?;
    let record = client
        .install(&store, &id, version)
        .map_err(registry_error)?;
    Ok(success(
        "package install",
        json!({
            "registry": registry,
            "registry_profile": registry_profile,
            "package": managed_package_value(&store, &record)?,
        }),
        format!(
            "Package {}@{} installed from the registry",
            record.manifest().id().as_str(),
            record.manifest().version()
        ),
    ))
}

fn validate(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["manifest", "artifact"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package validate does not accept positional arguments",
        ));
    }
    let manifest_path = required_path(&parsed, "manifest")?;
    let artifact_path = required_path(&parsed, "artifact")?;
    let manifest = read_manifest(&manifest_path)?;
    let artifact = read_artifact(&artifact_path)?;
    manifest
        .validate()
        .map_err(|error| CliError::usage(format!("package manifest is invalid: {error}")))?;
    if !matches!(
        manifest.kind(),
        PackageKind::Gene | PackageKind::DomainHarness | PackageKind::MetaHarness
    ) {
        return Err(CliError::execution(
            "package kind is not installable by the local runtime",
            json!({"kind": manifest.kind().as_str()}),
        ));
    }
    let actual_hash = hash_artifact(&artifact);
    if actual_hash != manifest.content_hash() {
        return Err(CliError::execution(
            "package artifact hash does not match its manifest",
            json!({
                "expected": manifest.content_hash(),
                "actual": actual_hash,
            }),
        ));
    }
    let execution_boundary = if manifest.kind() == PackageKind::Gene {
        WasmExecutor::new()
            .validate_artifact(&manifest, &artifact)
            .map_err(|error| {
                CliError::execution(
                    "Gene artifact is not a valid import-free Pandora WASM module",
                    json!({"error": error.to_string()}),
                )
            })?;
        "wasm"
    } else {
        "metadata-only"
    };
    Ok(success(
        "package validate",
        json!({
            "valid": true,
            "package": {
                "id": manifest.id(),
                "version": manifest.version(),
                "kind": manifest.kind().as_str(),
                "content_hash": manifest.content_hash(),
            },
            "execution_boundary": execution_boundary,
            "persisted": false,
        }),
        format!(
            "Validated {}@{} without persisting it",
            manifest.id(),
            manifest.version()
        ),
    ))
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
    let manifest = read_manifest(&manifest_path)?;
    let artifact = read_artifact(&artifact_path)?;
    let config = load_config(&parsed)?;
    let store =
        PackageStore::open(config.data_dir().join("packages.sqlite3")).map_err(store_error)?;
    let record = store
        .admit(&manifest, &manifest, &artifact)
        .map_err(store_error)?;
    let id = record.manifest().id().as_str().to_owned();
    Ok(success(
        "package admit",
        json!({"package": managed_package_value(&store, &record)?}),
        format!("Package {id}@{} admitted", record.manifest().version()),
    ))
}

fn read_manifest(path: &Path) -> Result<PackageManifest, CliError> {
    let manifest_bytes = fs::read(path).map_err(|error| {
        CliError::configuration(
            "could not read package manifest",
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    serde_json::from_slice(&manifest_bytes)
        .map_err(|error| CliError::usage(format!("package manifest is invalid: {error}")))
}

fn list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package list does not accept positional arguments",
        ));
    }
    let store = store(&parsed)?;
    let records = store.list().map_err(store_error)?;
    let count = records.len();
    let packages = records
        .iter()
        .map(|record| managed_package_value(&store, record))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(success(
        "package list",
        json!({
            "packages": packages
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
    Ok(success(
        "package inspect",
        json!({"package": managed_package_value(&store, &record)?}),
        format!("{}@{}", id.as_str(), version),
    ))
}

fn enable(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "dry-run", "yes"])?;
    if parsed.positionals.len() != 2 {
        return Err(CliError::usage(
            "package enable requires an ID and an exact version",
        ));
    }
    let dry_run = lifecycle_mode(&parsed, "package enable")?;
    let id = PackageId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("package ID is invalid"))?;
    let version = &parsed.positionals[1];
    let store = store(&parsed)?;
    let record = required_record(&store, &id, version)?;
    let before = store.binding(&id).map_err(store_error)?;
    let dependencies = dependency_preview(&store, &record)?;
    let blockers = before
        .as_ref()
        .and_then(|binding| binding.active_version())
        .filter(|active| *active != version)
        .map(|active| store.enabled_dependents(&id, active).map_err(store_error))
        .transpose()?
        .unwrap_or_default();
    if dry_run {
        let ready = dependencies
            .iter()
            .all(|dependency| dependency["optional"] == true || dependency["enabled"] == true)
            && blockers.is_empty();
        return Ok(success(
            "package enable",
            json!({
                "dry_run": true,
                "changed": false,
                "ready": ready,
                "package": managed_package_value(&store, &record)?,
                "dependencies": dependencies,
                "enabled_dependents": blockers,
            }),
            format!("Previewed activation for {}@{}", id.as_str(), version),
        ));
    }
    let binding = store.enable(&id, version).map_err(lifecycle_error)?;
    Ok(success(
        "package enable",
        json!({
            "dry_run": false,
            "changed": before.as_ref() != Some(&binding),
            "package": managed_package_value(&store, &record)?,
            "binding": binding_value(Some(&binding), version),
        }),
        format!(
            "Enabled {}@{} as the exact active version",
            id.as_str(),
            version
        ),
    ))
}

fn disable(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "dry-run", "yes"])?;
    if parsed.positionals.len() != 2 {
        return Err(CliError::usage(
            "package disable requires an ID and an exact version",
        ));
    }
    let dry_run = lifecycle_mode(&parsed, "package disable")?;
    let id = PackageId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("package ID is invalid"))?;
    let version = &parsed.positionals[1];
    let store = store(&parsed)?;
    let record = required_record(&store, &id, version)?;
    let before = store.binding(&id).map_err(store_error)?;
    let dependents = store
        .enabled_dependents(&id, version)
        .map_err(store_error)?;
    if dry_run {
        return Ok(success(
            "package disable",
            json!({
                "dry_run": true,
                "changed": false,
                "ready": before.as_ref().is_some_and(|binding| binding.enables(version)) && dependents.is_empty(),
                "package": managed_package_value(&store, &record)?,
                "enabled_dependents": dependents,
            }),
            format!("Previewed disable for {}@{}", id.as_str(), version),
        ));
    }
    let binding = store.disable(&id, version).map_err(lifecycle_error)?;
    Ok(success(
        "package disable",
        json!({
            "dry_run": false,
            "changed": true,
            "package": managed_package_value(&store, &record)?,
            "binding": binding_value(Some(&binding), version),
        }),
        format!(
            "Disabled {}@{} without removing its verified bytes",
            id.as_str(),
            version
        ),
    ))
}

fn rollback(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "dry-run", "yes"])?;
    if parsed.positionals.len() != 1 {
        return Err(CliError::usage(
            "package rollback requires exactly one package ID",
        ));
    }
    let dry_run = lifecycle_mode(&parsed, "package rollback")?;
    let id = PackageId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("package ID is invalid"))?;
    let store = store(&parsed)?;
    let before = store.binding(&id).map_err(store_error)?.ok_or_else(|| {
        lifecycle_error(PackageStoreError::NoRollbackBinding {
            id: id.as_str().to_owned(),
        })
    })?;
    let target = before.previous_version().ok_or_else(|| {
        lifecycle_error(PackageStoreError::NoRollbackBinding {
            id: id.as_str().to_owned(),
        })
    })?;
    let record = required_record(&store, &id, target)?;
    let dependencies = dependency_preview(&store, &record)?;
    let dependents = before
        .active_version()
        .map(|active| store.enabled_dependents(&id, active).map_err(store_error))
        .transpose()?
        .unwrap_or_default();
    if dry_run {
        let ready = dependencies
            .iter()
            .all(|dependency| dependency["optional"] == true || dependency["enabled"] == true)
            && dependents.is_empty();
        return Ok(success(
            "package rollback",
            json!({
                "dry_run": true,
                "changed": false,
                "ready": ready,
                "target_version": target,
                "dependencies": dependencies,
                "enabled_dependents": dependents,
                "binding": binding_value(Some(&before), target),
            }),
            format!("Previewed rollback of {} to {}", id.as_str(), target),
        ));
    }
    let binding = store.rollback(&id).map_err(lifecycle_error)?;
    let active = binding
        .active_version()
        .expect("rollback always restores an active version");
    Ok(success(
        "package rollback",
        json!({
            "dry_run": false,
            "changed": true,
            "active_version": active,
            "binding": binding_value(Some(&binding), active),
        }),
        format!("Rolled {} back to exact version {}", id.as_str(), active),
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
    let config = load_config(&parsed)?;
    let store =
        PackageStore::open(config.data_dir().join("packages.sqlite3")).map_err(store_error)?;
    let record = store
        .get(&id, version)
        .map_err(store_error)?
        .ok_or_else(|| {
            CliError::execution(
                "package was not admitted locally",
                json!({"id": id.as_str(), "version": version}),
            )
        })?;
    let artifact = ArtifactId::new(record.manifest().content_hash())
        .map_err(|_| CliError::internal("package artifact identity is invalid", json!({})))?;
    let catalog = ArtifactCatalog::open(config.data_dir().join("artifact-catalog.sqlite3"))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let bindings = catalog
        .references(&artifact)
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    if !bindings.is_empty() {
        return Err(CliError::execution(
            "package artifact is referenced by an active evolution binding",
            json!({"id": id.as_str(), "version": version, "artifact": artifact, "proposals": bindings.iter().map(|binding| binding.proposal_id()).collect::<Vec<_>>() }),
        ));
    }
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

fn lifecycle_mode(parsed: &super::ParsedArgs, command: &str) -> Result<bool, CliError> {
    let dry_run = parsed.value("dry-run").is_some();
    let confirmed = parsed.value("yes").is_some();
    if dry_run && confirmed {
        return Err(CliError::usage(format!(
            "{command} accepts only one of '--dry-run' or '--yes'"
        )));
    }
    if !dry_run && !confirmed {
        return Err(CliError::usage(format!(
            "{command} requires '--dry-run' or '--yes'"
        )));
    }
    Ok(dry_run)
}

fn required_record(
    store: &PackageStore,
    id: &PackageId,
    version: &str,
) -> Result<PackageRecord, CliError> {
    store.get(id, version).map_err(store_error)?.ok_or_else(|| {
        CliError::execution(
            "package was not admitted locally",
            json!({"id": id.as_str(), "version": version}),
        )
    })
}

fn dependency_preview(
    store: &PackageStore,
    record: &PackageRecord,
) -> Result<Vec<Value>, CliError> {
    let mut dependencies = Vec::new();
    for dependency in record.manifest().dependencies() {
        let installed = store
            .get(dependency.id(), dependency.version())
            .map_err(store_error)?;
        let built_in = builtin_genes().into_iter().any(|gene| {
            gene.manifest().id().as_str() == dependency.id().as_str()
                && gene.manifest().version() == dependency.version()
        });
        let enabled = if installed.is_some() {
            store
                .is_enabled(dependency.id(), dependency.version())
                .map_err(store_error)?
        } else {
            built_in
        };
        dependencies.push(json!({
            "id": dependency.id().as_str(),
            "version": dependency.version(),
            "optional": dependency.optional(),
            "source": if built_in { "built_in" } else if installed.is_some() { "package" } else { "unresolved" },
            "enabled": enabled,
        }));
    }
    if let Some(composition) = record.manifest().meta_composition() {
        let records = store.list().map_err(store_error)?;
        for domain in composition.allowed_domains() {
            let built_in = builtin_harnesses().into_iter().find(|harness| {
                harness.manifest().id() == domain
                    && harness.manifest().kind() == pandora_types::HarnessKind::Domain
            });
            let package_id = PackageId::new(domain.as_str().to_owned())
                .map_err(|_| CliError::internal("Domain package identity is invalid", json!({})))?;
            let binding = store.binding(&package_id).map_err(store_error)?;
            let active_version = binding.as_ref().and_then(PackageBinding::active_version);
            let packaged = active_version.is_some_and(|version| {
                records.iter().any(|candidate| {
                    candidate.manifest().id().as_str() == domain.as_str()
                        && candidate.manifest().version() == version
                        && candidate.manifest().kind() == PackageKind::DomainHarness
                })
            });
            dependencies.push(json!({
                "id": domain.as_str(),
                "version": built_in.as_ref().map(|harness| harness.manifest().version()).or(active_version),
                "optional": false,
                "source": if built_in.is_some() { "built_in" } else if packaged { "package" } else { "unresolved" },
                "enabled": built_in.is_some() || packaged,
            }));
        }
    }
    Ok(dependencies)
}

pub(super) fn managed_package_value(
    store: &PackageStore,
    record: &PackageRecord,
) -> Result<Value, CliError> {
    let binding = store.binding(record.manifest().id()).map_err(store_error)?;
    let mut value = package_value(record);
    value["activation"] = binding_value(binding.as_ref(), record.manifest().version());
    Ok(value)
}

fn binding_value(binding: Option<&PackageBinding>, version: &str) -> Value {
    json!({
        "state": if binding.is_some_and(|binding| binding.enables(version)) { "enabled" } else { "disabled" },
        "active_version": binding.and_then(PackageBinding::active_version),
        "previous_version": binding.and_then(PackageBinding::previous_version),
        "generation": binding.map_or(0, PackageBinding::generation),
        "runtime_authority": false,
    })
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
        PackageStoreError::PackageBound { id, version, role } => CliError::policy(
            format!(
                "cannot remove package {id}@{version} while it is the {role} lifecycle binding"
            ),
            json!({"id": id, "version": version, "role": role}),
        ),
        other => store_error(other),
    }
}

fn lifecycle_error(error: PackageStoreError) -> CliError {
    match error {
        PackageStoreError::HasEnabledDependents {
            id,
            version,
            dependents,
        } => CliError::policy(
            format!("cannot change package {id}@{version} while enabled dependents exist"),
            json!({"id": id, "version": version, "enabled_dependents": dependents}),
        ),
        PackageStoreError::MissingEnabledDependency { id, version } => CliError::policy(
            format!("required package dependency {id}@{version} must be enabled first"),
            json!({"id": id, "version": version}),
        ),
        PackageStoreError::MissingEnabledDomain { id } => CliError::policy(
            format!("required Domain Harness {id} must be enabled first"),
            json!({"id": id}),
        ),
        PackageStoreError::PackageBound { id, version, role } => CliError::policy(
            format!("package {id}@{version} is retained as the {role} lifecycle binding"),
            json!({"id": id, "version": version, "role": role}),
        ),
        other => store_error(other),
    }
}

fn registry_error(error: PackageRegistryError) -> CliError {
    CliError::execution(error.to_string(), json!({}))
}

fn github_error(error: GitHubPackageError) -> CliError {
    CliError::execution(error.to_string(), json!({}))
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
