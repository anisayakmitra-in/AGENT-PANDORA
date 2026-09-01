use super::{load_config, parse_options, timestamp, write_config};
use crate::output::{CliError, CommandResult, success};
use atomic_write_file::AtomicWriteFile;
use ed25519_dalek::{Signer, SigningKey};
use pandora_harnesses::{builtin_genes, builtin_harnesses, replaceable_builtin_harness_kind};
use pandora_provider::ProviderManifest;
use pandora_runtime::config::DEFAULT_REGISTRY_TOKEN_ENV;
use pandora_runtime::config::ProviderProfile;
use pandora_runtime::skill_engine::SkillEngine;
use pandora_runtime::{
    ArtifactCatalog, DistributionEvent, DistributionRecord, DistributionSource,
    DistributionSourceKind, GitHubPackageClient, GitHubPackageError, MAX_DISTRIBUTION_LIST,
    MAX_PACKAGE_TRANSPARENCY_LIST, MAX_STORED_ARTIFACT_BYTES, PackageBinding,
    PackageDistributionError, PackageDistributionStore, PackageRecord, PackageRegistryClient,
    PackageRegistryError, PackageStore, PackageStoreError, PackageTransparencyEvent,
    PackageTransparencyEventKind, PackageTransparencyOutcome, WasmExecutor,
    materialize_skill_bundle,
};
use pandora_types::{
    ArtifactId, DomainRoutingProfile, HarnessId, MAX_META_HANDOFFS, MetaComposition,
    PackageCompatibility, PackageDependency, PackageId, PackageKind, PackageManifest,
    TrustEvidence, hash_artifact,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

const DEFAULT_GITHUB_TOKEN_ENV: &str = "PANDORA_GITHUB_TOKEN";

pub fn execute(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args.first().ok_or_else(|| {
        CliError::usage(
            "package requires 'scaffold', 'admit', 'admit-cached', 'validate', 'sign', 'keygen', 'discover', 'download', 'download-github', 'install', 'install-github', 'cache', 'list', 'inspect', 'enable', 'disable', 'rollback', 'lock', 'verify-lock', 'trust-root', 'transparency', or 'remove'",
        )
    })?;
    match subcommand.as_str() {
        "scaffold" => scaffold(&args[1..]),
        "admit" => admit(&args[1..]),
        "admit-cached" => admit_cached(&args[1..]),
        "validate" => validate(&args[1..]),
        "sign" => sign(&args[1..]),
        "keygen" => keygen(&args[1..]),
        "discover" => discover(&args[1..]),
        "download" | "install" => download(&args[1..]),
        "download-github" | "install-github" => install_github(&args[1..]),
        "cache" => cache(&args[1..]),
        "list" => list(&args[1..]),
        "inspect" => inspect(&args[1..]),
        "enable" => enable(&args[1..]),
        "disable" => disable(&args[1..]),
        "rollback" => rollback(&args[1..]),
        "lock" => lock(&args[1..]),
        "verify-lock" => verify_lock(&args[1..]),
        "trust-root" => trust_root(&args[1..]),
        "transparency" => transparency(&args[1..]),
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
            "package download-github does not accept positional arguments",
        ));
    }
    let repository = parsed.value("repository").ok_or_else(|| {
        CliError::usage("package download-github requires '--repository <GitHub URL>'")
    })?;
    let commit = parsed
        .value("commit")
        .ok_or_else(|| CliError::usage("package download-github requires '--commit <full SHA>'"))?;
    let manifest_path = parsed.value("manifest").ok_or_else(|| {
        CliError::usage("package download-github requires '--manifest <repository path>'")
    })?;
    let artifact_path = parsed.value("artifact").ok_or_else(|| {
        CliError::usage("package download-github requires '--artifact <repository path>'")
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
    let download = client
        .download(manifest_path, artifact_path)
        .map_err(github_error)?;
    let (manifest, artifact) = download.into_parts();
    let config = load_config(&parsed)?;
    let distribution = distribution_store(&config)?;
    let source = DistributionSource::new(
        DistributionSourceKind::GitHub,
        repository,
        commit.to_ascii_lowercase(),
    )
    .map_err(distribution_error)?;
    let cached = distribution
        .cache_verified(&manifest, &artifact, source, timestamp().as_unix_seconds())
        .map_err(distribution_error)?;
    Ok(success(
        "package download-github",
        json!({
            "source": {
                "kind": "github",
                "repository": repository,
                "commit": commit.to_ascii_lowercase(),
                "manifest_path": manifest_path,
                "artifact_path": artifact_path,
            },
            "changed": cached.changed(),
            "package": distribution_record_value(&distribution, cached.record())?,
        }),
        format!(
            "Package {}@{} downloaded and verified from the pinned GitHub source; admission and enablement are unchanged",
            manifest.id().as_str(),
            manifest.version()
        ),
    ))
}

fn download(args: &[String]) -> Result<CommandResult, CliError> {
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
            "package download requires an ID and accepts one optional exact version",
        ));
    }
    let id = PackageId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("package ID is invalid"))?;
    let requested_version = parsed.positionals.get(1).map(String::as_str);
    if let Some(version) = requested_version {
        Version::parse(version)
            .map_err(|_| CliError::usage("package version must be exact SemVer"))?;
    }
    let (config, registry, registry_profile, token) = registry_connection(&parsed)?;
    let client = PackageRegistryClient::new(&registry, token).map_err(registry_error)?;
    let resolved_version = match requested_version {
        Some(version) => version.to_owned(),
        None => client
            .discover(&id, None)
            .map_err(registry_error)?
            .version()
            .to_owned(),
    };
    let download = client
        .download_exact(&id, &resolved_version)
        .map_err(registry_error)?;
    let (manifest, artifact) = download.into_parts();
    let distribution = distribution_store(&config)?;
    let source = DistributionSource::new(
        DistributionSourceKind::Registry,
        &registry,
        &resolved_version,
    )
    .map_err(distribution_error)?;
    let cached = distribution
        .cache_verified(&manifest, &artifact, source, timestamp().as_unix_seconds())
        .map_err(distribution_error)?;
    Ok(success(
        "package download",
        json!({
            "registry": registry,
            "registry_profile": registry_profile,
            "changed": cached.changed(),
            "package": distribution_record_value(&distribution, cached.record())?,
        }),
        format!(
            "Package {}@{} downloaded and verified; admission and enablement are unchanged",
            manifest.id().as_str(),
            manifest.version()
        ),
    ))
}

fn discover(args: &[String]) -> Result<CommandResult, CliError> {
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
            "package discover requires an ID and accepts one optional exact version",
        ));
    }
    let id = PackageId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("package ID is invalid"))?;
    let version = parsed.positionals.get(1).map(String::as_str);
    if let Some(version) = version {
        Version::parse(version)
            .map_err(|_| CliError::usage("package version must be exact SemVer"))?;
    }
    let (config, registry, profile, token) = registry_connection(&parsed)?;
    let client = PackageRegistryClient::new(&registry, token).map_err(registry_error)?;
    let manifest = client.discover(&id, version).map_err(registry_error)?;
    Ok(success(
        "package discover",
        json!({
            "registry": registry,
            "registry_profile": profile,
            "resolved_version": manifest.version(),
            "package": manifest,
            "downloaded": false,
            "admitted": false,
            "enabled": false,
            "data_dir": config.data_dir(),
        }),
        format!(
            "Resolved {}@{} without downloading, admitting, or enabling it",
            id,
            manifest.version()
        ),
    ))
}

fn cache(args: &[String]) -> Result<CommandResult, CliError> {
    let (command, rest) = args.split_first().ok_or_else(|| {
        CliError::usage("package cache requires 'list', 'inspect', 'verify', or 'events'")
    })?;
    match command.as_str() {
        "list" => cache_list(rest),
        "inspect" => cache_inspect(rest),
        "verify" => cache_verify(rest),
        "events" => cache_events(rest),
        unknown => Err(CliError::usage(format!(
            "unknown package cache command '{unknown}'"
        ))),
    }
}

fn cache_list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "limit"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package cache list does not accept positional arguments",
        ));
    }
    let limit = distribution_limit(parsed.value("limit"))?;
    let config = load_config(&parsed)?;
    let store = distribution_store(&config)?;
    let packages = store
        .list(limit)
        .map_err(distribution_error)?
        .iter()
        .map(|record| distribution_record_value(&store, record))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(success(
        "package cache list",
        json!({
            "packages": packages,
            "count": packages.len(),
            "download_authority": "cache_only",
            "admission_performed": false,
            "enablement_performed": false,
        }),
        format!("{} verified package download(s) cached", packages.len()),
    ))
}

fn cache_inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let (id, version) = distribution_identity(&parsed, "package cache inspect")?;
    let config = load_config(&parsed)?;
    let store = distribution_store(&config)?;
    let record = store
        .get(&id, &version)
        .map_err(distribution_error)?
        .ok_or_else(|| distribution_error(PackageDistributionError::NotFound))?;
    let events = store
        .list_events(MAX_DISTRIBUTION_LIST)
        .map_err(distribution_error)?
        .into_iter()
        .filter(|event| event.package_id() == id.as_str() && event.package_version() == version)
        .map(|event| distribution_event_value(&event))
        .collect::<Vec<_>>();
    Ok(success(
        "package cache inspect",
        json!({
            "package": distribution_record_value(&store, &record)?,
            "events": events,
            "artifact_bytes": record.artifact().len(),
            "artifact_exposed": false,
        }),
        format!("Inspected cached package {id}@{version}"),
    ))
}

fn cache_verify(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    let (id, version) = distribution_identity(&parsed, "package cache verify")?;
    let config = load_config(&parsed)?;
    let store = distribution_store(&config)?;
    let record = store
        .verify_offline(&id, &version, timestamp().as_unix_seconds())
        .map_err(distribution_error)?;
    Ok(success(
        "package cache verify",
        json!({
            "package": distribution_record_value(&store, &record)?,
            "verification": "verified_offline",
            "network_used": false,
            "admission_performed": false,
            "enablement_performed": false,
        }),
        format!("Verified {id}@{version} from the local cache without network access"),
    ))
}

fn cache_events(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "limit"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package cache events does not accept positional arguments",
        ));
    }
    let limit = distribution_limit(parsed.value("limit"))?;
    let config = load_config(&parsed)?;
    let events = distribution_store(&config)?
        .list_events(limit)
        .map_err(distribution_error)?
        .iter()
        .map(distribution_event_value)
        .collect::<Vec<_>>();
    Ok(success(
        "package cache events",
        json!({
            "events": events,
            "count": events.len(),
            "durability": "append-only-sqlite",
            "integrity": "sha256-event-chain",
        }),
        format!("Loaded {} package distribution event(s)", events.len()),
    ))
}

fn admit_cached(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "dry-run", "yes"])?;
    let (id, version) = distribution_identity(&parsed, "package admit-cached")?;
    let dry_run = parsed.value("dry-run").is_some();
    let confirmed = parsed.value("yes").is_some();
    if dry_run == confirmed {
        return Err(CliError::usage(
            "package admit-cached requires exactly one of '--dry-run' or '--yes'",
        ));
    }
    let mut config = load_config(&parsed)?;
    let distribution = distribution_store(&config)?;
    let record = distribution
        .prepare_admission(&id, &version)
        .map_err(distribution_error)?;
    if dry_run {
        return Ok(success(
            "package admit-cached",
            json!({
                "dry_run": true,
                "package": distribution_record_value(&distribution, &record)?,
                "admission_boundary": package_admission_boundary(record.manifest().kind()),
                "enablement_performed": false,
                "effect_authority_granted": false,
            }),
            format!("Verified the admission plan for {id}@{version}; no local boundary changed"),
        ));
    }

    let already_admitted = record.state().as_str() == "admitted"
        && distribution
            .binding(&id)
            .map_err(distribution_error)?
            .is_some_and(|binding| binding.active_version() == version);
    if !already_admitted {
        match record.manifest().kind() {
            PackageKind::Gene | PackageKind::DomainHarness | PackageKind::MetaHarness => {
                admit_cached_harness(&config, &record)?;
            }
            PackageKind::Skill => {
                admit_cached_skill(&config, &record)?;
            }
            PackageKind::Provider => {
                admit_cached_provider(&mut config, &record)?;
                write_config(&config)?;
            }
            kind => {
                return Err(CliError::execution(
                    "cached package kind has no local admission boundary",
                    json!({"kind": kind.as_str()}),
                ));
            }
        }
    }
    let admitted = distribution
        .record_admission(
            &id,
            &version,
            &record.artifact_digest(),
            timestamp().as_unix_seconds(),
        )
        .map_err(distribution_error)?;
    Ok(success(
        "package admit-cached",
        json!({
            "dry_run": false,
            "changed": admitted.changed(),
            "package": distribution_record_value(&distribution, admitted.record())?,
            "admission_boundary": package_admission_boundary(record.manifest().kind()),
            "enablement_performed": false,
            "effect_authority_granted": false,
        }),
        format!(
            "Package {id}@{version} admitted to its local boundary but left disabled or inactive"
        ),
    ))
}

fn admit_cached_harness(
    config: &pandora_runtime::config::RuntimeConfig,
    record: &DistributionRecord,
) -> Result<(), CliError> {
    let store =
        PackageStore::open(config.data_dir().join("packages.sqlite3")).map_err(store_error)?;
    if let Some(existing) = store
        .get(record.manifest().id(), record.manifest().version())
        .map_err(store_error)?
    {
        if existing.manifest() == record.manifest() {
            return Ok(());
        }
        return Err(CliError::execution(
            "local package identity conflicts with the cached artifact",
            json!({}),
        ));
    }
    store
        .admit(record.manifest(), record.manifest(), record.artifact())
        .map_err(store_error)?;
    Ok(())
}

fn admit_cached_skill(
    config: &pandora_runtime::config::RuntimeConfig,
    record: &DistributionRecord,
) -> Result<(), CliError> {
    let skill_id = package_leaf_id(record.manifest().id())?;
    let skills_root = config.data_dir().join("skills");
    fs::create_dir_all(&skills_root).map_err(|_| {
        CliError::configuration(
            "could not create the skill directory",
            json!({"root": skills_root}),
        )
    })?;
    let engine = SkillEngine::discover(&skills_root)
        .map_err(|error| CliError::execution(error.to_string(), json!({})))?;
    if let Ok(existing) = engine.inspect(skill_id) {
        if existing.manifest().version() == record.manifest().version()
            && existing.manifest().publisher() == Some(record.manifest().publisher())
        {
            return Ok(());
        }
        return Err(CliError::execution(
            "a different Skill version is already admitted; remove it before applying this update",
            json!({
                "skill_id": skill_id,
                "installed_version": existing.manifest().version(),
                "requested_version": record.manifest().version(),
            }),
        ));
    }
    let staging_parent = config.data_dir().join("package-admission-staging");
    fs::create_dir_all(&staging_parent).map_err(|_| {
        CliError::configuration("could not create package admission staging", json!({}))
    })?;
    let suffix = record.artifact_digest().replace(':', "-");
    let staging_root = staging_parent.join(format!(
        "{}-{}-{}",
        std::process::id(),
        timestamp().as_unix_seconds(),
        &suffix[..suffix.len().min(24)]
    ));
    let skill_root = materialize_skill_bundle(record.artifact(), &staging_root, skill_id)
        .map_err(distribution_error)?;
    let result = (|| {
        let manifest = engine
            .validate_source(&skill_root)
            .map_err(|error| CliError::execution(error.to_string(), json!({})))?;
        if manifest.id().as_str() != skill_id
            || manifest.version() != record.manifest().version()
            || manifest.publisher() != Some(record.manifest().publisher())
        {
            return Err(CliError::execution(
                "Skill bundle identity does not match its signed package manifest",
                json!({
                    "package_id": record.manifest().id().as_str(),
                    "skill_id": manifest.id().as_str(),
                    "package_version": record.manifest().version(),
                    "skill_version": manifest.version(),
                }),
            ));
        }
        engine
            .install_from(&skill_root)
            .map_err(|error| CliError::execution(error.to_string(), json!({})))?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging_root);
    result
}

fn admit_cached_provider(
    config: &mut pandora_runtime::config::RuntimeConfig,
    record: &DistributionRecord,
) -> Result<(), CliError> {
    let provider: ProviderManifest = serde_json::from_slice(record.artifact()).map_err(|_| {
        CliError::execution(
            "Provider package artifact is not valid provider JSON",
            json!({}),
        )
    })?;
    let provider_id = package_leaf_id(record.manifest().id())?;
    if provider.id().as_str() != provider_id {
        return Err(CliError::execution(
            "Provider artifact identity does not match its signed package manifest",
            json!({
                "package_id": record.manifest().id().as_str(),
                "provider_id": provider.id().as_str(),
            }),
        ));
    }
    let profile = ProviderProfile::new_with_protocol(
        provider.id().as_str(),
        provider.protocol(),
        provider.base_url(),
        provider.default_model().as_str(),
        provider.api_key_env(),
    )
    .map_err(|error| CliError::configuration(error.to_string(), json!({})))?;
    if config.active_provider() == Some(provider_id)
        && config.provider_profile(provider_id) != Some(&profile)
    {
        return Err(CliError::execution(
            "an active Provider cannot be replaced during admission; select another Provider first",
            json!({"provider": provider_id}),
        ));
    }
    config.admit_provider_profile(profile);
    Ok(())
}

#[derive(Deserialize, Serialize)]
struct StoredPackageSigningKey {
    format_version: u32,
    publisher: String,
    key_id: String,
    public_key: String,
    private_key: String,
}

fn keygen(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "publisher",
            "key-id",
            "secret-name",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package keygen does not accept positional arguments",
        ));
    }
    let publisher = parsed
        .value("publisher")
        .ok_or_else(|| CliError::usage("package keygen requires '--publisher <name>'"))?;
    let key_id = parsed
        .value("key-id")
        .ok_or_else(|| CliError::usage("package keygen requires '--key-id <id>'"))?;
    let secret_name = parsed
        .value("secret-name")
        .ok_or_else(|| CliError::usage("package keygen requires '--secret-name <vault-name>'"))?;
    let config = load_config(&parsed)?;
    let mut vault = super::secret::open_vault(&config)?;
    if vault
        .get(secret_name)
        .map_err(super::secret::vault_error)?
        .is_some()
    {
        return Err(CliError::configuration(
            "the package signing secret already exists",
            json!({"secret_name": secret_name}),
        ));
    }

    let mut private_key = [0_u8; 32];
    getrandom::fill(&mut private_key).map_err(|_| {
        CliError::configuration("could not generate a package signing key", json!({}))
    })?;
    let signing_key = SigningKey::from_bytes(&private_key);
    let public_key = encode_hex(&signing_key.verifying_key().to_bytes());
    let private_key_hex = encode_hex(&private_key);
    private_key.zeroize();
    let stored = StoredPackageSigningKey {
        format_version: 1,
        publisher: publisher.to_owned(),
        key_id: key_id.to_owned(),
        public_key: public_key.clone(),
        private_key: private_key_hex,
    };
    let secret = serde_json::to_string(&stored)
        .map_err(|_| CliError::internal("could not encode package signing key", json!({})))?;
    let entry = vault
        .put(secret_name, secret, super::timestamp().as_unix_seconds())
        .map_err(super::secret::vault_error)?;
    Ok(success(
        "package keygen",
        json!({
            "publisher": publisher,
            "key_id": key_id,
            "secret_name": entry.name(),
            "public_key": public_key,
            "private_key_exposed": false,
            "stored": true,
            "vault_path": vault.path(),
        }),
        format!(
            "Generated a local package signing key for {publisher}; private material remains in the encrypted vault"
        ),
    ))
}

fn sign(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "manifest",
            "artifact",
            "secret-name",
            "output",
            "yes",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package sign does not accept positional arguments",
        ));
    }
    let manifest_path = command_path(&parsed, "manifest", "package sign")?;
    let artifact_path = command_path(&parsed, "artifact", "package sign")?;
    let output_path = command_path(&parsed, "output", "package sign")?;
    if manifest_path == output_path {
        return Err(CliError::usage(
            "package sign requires an output path different from the input manifest",
        ));
    }
    if let Ok(metadata) = fs::symlink_metadata(&output_path) {
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(CliError::configuration(
                "package sign output path is unsafe",
                json!({"path": output_path}),
            ));
        }
        if parsed.value("yes").is_none() {
            return Err(CliError::usage(
                "package sign refuses to overwrite an existing output without '--yes'",
            ));
        }
    }

    let manifest = read_manifest(&manifest_path)?;
    manifest
        .validate()
        .map_err(|error| CliError::usage(format!("package manifest is invalid: {error}")))?;
    let artifact = read_artifact(&artifact_path)?;
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
    let secret_name = parsed
        .value("secret-name")
        .ok_or_else(|| CliError::usage("package sign requires '--secret-name <vault-name>'"))?;
    let config = load_config(&parsed)?;
    let vault = super::secret::open_vault(&config)?;
    let secret = vault
        .get(secret_name)
        .map_err(super::secret::vault_error)?
        .ok_or_else(|| {
            CliError::configuration(
                "package signing secret is not configured",
                json!({"secret_name": secret_name}),
            )
        })?;
    let mut stored: StoredPackageSigningKey =
        serde_json::from_str(secret.expose()).map_err(|_| {
            CliError::configuration(
                "package signing secret has an invalid format",
                json!({"secret_name": secret_name}),
            )
        })?;
    if stored.format_version != 1
        || stored.publisher != manifest.publisher()
        || stored.key_id.trim().is_empty()
    {
        stored.private_key.zeroize();
        return Err(CliError::configuration(
            "package signing key does not match the manifest publisher",
            json!({
                "secret_name": secret_name,
                "manifest_publisher": manifest.publisher(),
            }),
        ));
    }
    let mut private_key = match decode_private_key(&stored.private_key) {
        Ok(key) => key,
        Err(error) => {
            stored.private_key.zeroize();
            return Err(error);
        }
    };
    stored.private_key.zeroize();
    let signing_key = SigningKey::from_bytes(&private_key);
    private_key.zeroize();
    let public_key = encode_hex(&signing_key.verifying_key().to_bytes());
    if public_key != stored.public_key {
        return Err(CliError::configuration(
            "package signing key public identity does not match its stored evidence",
            json!({"secret_name": secret_name}),
        ));
    }
    let signature = signing_key.sign(manifest.signing_message().as_bytes());
    let mut signed_value = serde_json::to_value(&manifest)
        .map_err(|_| CliError::internal("could not encode package manifest", json!({})))?;
    signed_value["trust"] = json!({
        "level": "verified",
        "signature": encode_hex(&signature.to_bytes()),
        "public_key": public_key,
    });
    let signed_manifest: PackageManifest = serde_json::from_value(signed_value).map_err(|_| {
        CliError::internal("could not rebuild the signed package manifest", json!({}))
    })?;
    signed_manifest.validate().map_err(|error| {
        CliError::internal(
            format!("signed package manifest is invalid: {error}"),
            json!({}),
        )
    })?;
    let encoded = serde_json::to_vec_pretty(&signed_manifest).map_err(|_| {
        CliError::internal("could not serialize signed package manifest", json!({}))
    })?;
    let mut file = AtomicWriteFile::open(&output_path).map_err(|error| {
        CliError::configuration(
            "could not open signed package manifest output",
            json!({"path": output_path, "error": error.to_string()}),
        )
    })?;
    file.write_all(&encoded).map_err(|error| {
        CliError::configuration(
            "could not write signed package manifest output",
            json!({"path": output_path, "error": error.to_string()}),
        )
    })?;
    file.commit().map_err(|error| {
        CliError::configuration(
            "could not commit signed package manifest output",
            json!({"path": output_path, "error": error.to_string()}),
        )
    })?;
    Ok(success(
        "package sign",
        json!({
            "manifest": output_path,
            "package": {
                "id": signed_manifest.id(),
                "version": signed_manifest.version(),
                "publisher": signed_manifest.publisher(),
                "content_hash": signed_manifest.content_hash(),
            },
            "key_id": stored.key_id,
            "public_key": signed_manifest.trust().public_key(),
            "signature_present": signed_manifest.trust().signature().is_some(),
            "private_key_exposed": false,
            "vault_secret": secret_name,
        }),
        format!(
            "Signed {}@{} into {}",
            signed_manifest.id(),
            signed_manifest.version(),
            output_path.display()
        ),
    ))
}

fn decode_private_key(value: &str) -> Result<[u8; 32], CliError> {
    if value.len() != 64 {
        return Err(CliError::configuration(
            "package signing secret must contain a 32-byte hexadecimal private key",
            json!({}),
        ));
    }
    let mut decoded = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let high = hex_digit(chunk[0]).ok_or_else(|| {
            CliError::configuration(
                "package signing secret contains invalid hexadecimal",
                json!({}),
            )
        })?;
        let low = hex_digit(chunk[1]).ok_or_else(|| {
            CliError::configuration(
                "package signing secret contains invalid hexadecimal",
                json!({}),
            )
        })?;
        decoded[index] = (high << 4) | low;
    }
    Ok(decoded)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

#[derive(Serialize)]
struct DomainHarnessStarterGene<'a> {
    id: &'a str,
    version: &'a str,
}

#[derive(Serialize)]
struct DomainHarnessStarterArtifact<'a> {
    format_version: u32,
    kind: &'static str,
    id: &'a str,
    version: &'a str,
    owned_genes: Vec<DomainHarnessStarterGene<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    route_hints: Vec<&'a str>,
}

#[derive(Clone, Serialize)]
struct MetaHarnessStarterDomain {
    id: String,
    version: String,
}

#[derive(Serialize)]
struct MetaHarnessStarterArtifact<'a> {
    format_version: u32,
    kind: &'static str,
    id: &'a str,
    version: &'a str,
    allowed_domains: Vec<MetaHarnessStarterDomain>,
    max_handoffs: u32,
}

fn scaffold(args: &[String]) -> Result<CommandResult, CliError> {
    let (kind, args) = args.split_first().ok_or_else(|| {
        CliError::usage(
            "package scaffold requires the 'domain-harness' or 'meta-harness' starter kind",
        )
    })?;
    match kind.as_str() {
        "domain-harness" => scaffold_domain_harness(args),
        "meta-harness" => scaffold_meta_harness(args),
        _ => Err(CliError::usage(format!(
            "unsupported package scaffold kind '{kind}'; expected domain-harness or meta-harness"
        ))),
    }
}

fn scaffold_domain_harness(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "output",
            "id",
            "version",
            "publisher",
            "gene",
            "route-hint",
            "license",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package scaffold domain-harness accepts only named options",
        ));
    }

    let output = required_path(&parsed, "output")?;
    let id = parsed.value("id").unwrap_or("example/domain-starter");
    let version = parsed.value("version").unwrap_or("1.0.0");
    let publisher = parsed.value("publisher").unwrap_or("local-contributor");
    let license = parsed.value("license").unwrap_or("Apache-2.0");
    let gene = parsed.value("gene").unwrap_or("workspace.read@0.1.0");
    let (gene_id, gene_version) = gene
        .rsplit_once('@')
        .ok_or_else(|| CliError::usage("--gene requires an exact '<gene-id>@<semver>' identity"))?;
    if gene_id.is_empty() || gene_version.is_empty() {
        return Err(CliError::usage(
            "--gene requires an exact '<gene-id>@<semver>' identity",
        ));
    }
    let route_hint = parsed.value("route-hint");

    let artifact_value = DomainHarnessStarterArtifact {
        format_version: 1,
        kind: "domain_harness_profile",
        id,
        version,
        owned_genes: vec![DomainHarnessStarterGene {
            id: gene_id,
            version: gene_version,
        }],
        route_hints: route_hint.into_iter().collect(),
    };
    let mut artifact = serde_json::to_vec_pretty(&artifact_value).map_err(|_| {
        CliError::internal(
            "could not encode Domain Harness starter artifact",
            json!({}),
        )
    })?;
    artifact.push(b'\n');

    let dependency = PackageDependency::new(gene_id, gene_version, false)
        .map_err(|error| CliError::usage(format!("starter Gene identity is invalid: {error}")))?;
    let compatibility = PackageCompatibility::new(format!("pandora={}", env!("CARGO_PKG_VERSION")))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let mut manifest = PackageManifest::new(
        id,
        version,
        PackageKind::DomainHarness,
        publisher,
        hash_artifact(&artifact),
        vec![dependency],
        compatibility,
        license,
        TrustEvidence::unsigned(),
    )
    .map_err(|error| CliError::usage(format!("starter manifest is invalid: {error}")))?;
    if let Some(hint) = route_hint {
        let routing = DomainRoutingProfile::new(vec![hint.to_owned()])
            .map_err(|error| CliError::usage(format!("starter route hint is invalid: {error}")))?;
        manifest = manifest
            .with_domain_routing(routing)
            .map_err(|error| CliError::usage(format!("starter routing is invalid: {error}")))?;
    }

    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|_| CliError::internal("could not encode starter manifest", json!({})))?;
    manifest_bytes.push(b'\n');
    let readme = domain_harness_starter_readme(id, version, gene_id, gene_version);
    let architecture = domain_harness_starter_architecture();
    let files = [
        ("pandora.package.json", manifest_bytes.as_slice()),
        ("domain-harness.artifact", artifact.as_slice()),
        ("README.md", readme.as_bytes()),
        ("ARCHITECTURE.md", architecture.as_bytes()),
    ];
    write_scaffold_directory(&output, &files)?;

    let manifest_path = output.join("pandora.package.json");
    let artifact_path = output.join("domain-harness.artifact");
    Ok(success(
        "package scaffold",
        json!({
            "scaffold": {
                "format_version": 1,
                "kind": "domain_harness",
                "directory": output,
                "manifest": manifest_path,
                "artifact": artifact_path,
                "package": {
                    "id": manifest.id().as_str(),
                    "version": manifest.version(),
                    "content_hash": manifest.content_hash(),
                    "runtime_compatibility": manifest.compatibility().runtime(),
                    "owned_genes": manifest.dependencies().iter().map(|dependency| json!({
                        "id": dependency.id().as_str(),
                        "version": dependency.version(),
                    })).collect::<Vec<_>>(),
                    "route_hints": manifest.domain_routing().map(|routing| routing.hints()).unwrap_or(&[]),
                },
            },
            "network_requested": false,
            "credential_accessed": false,
            "persisted_package": false,
            "runtime_authority": false,
            "next_steps": [
                ["pandora", "package", "validate", "--manifest", "pandora.package.json", "--artifact", "domain-harness.artifact"],
                ["pandora", "package", "admit", "--manifest", "pandora.package.json", "--artifact", "domain-harness.artifact"],
                ["pandora", "package", "enable", manifest.id().as_str(), manifest.version(), "--dry-run"],
                ["pandora", "package", "enable", manifest.id().as_str(), manifest.version(), "--yes"],
            ],
        }),
        format!(
            "Created a local Domain Harness starter at {} without admission or activation",
            output.display()
        ),
    ))
}

fn scaffold_meta_harness(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "output",
            "id",
            "version",
            "publisher",
            "domains",
            "max-handoffs",
            "license",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package scaffold meta-harness accepts only named options",
        ));
    }

    let output = required_path(&parsed, "output")?;
    let id = parsed.value("id").unwrap_or("example/meta-starter");
    let version = parsed.value("version").unwrap_or("1.0.0");
    let publisher = parsed.value("publisher").unwrap_or("local-contributor");
    let license = parsed.value("license").unwrap_or("Apache-2.0");
    let domains = parsed
        .value("domains")
        .unwrap_or("coding-domain@0.1.0,research-domain@0.1.0");
    let max_handoffs = parsed
        .value("max-handoffs")
        .unwrap_or("4")
        .parse::<u32>()
        .map_err(|_| {
            CliError::usage(format!(
                "--max-handoffs requires an integer between 1 and {MAX_META_HANDOFFS}"
            ))
        })?;

    let mut exact_domains = Vec::new();
    for token in domains
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let (domain_id, domain_version) = token.rsplit_once('@').ok_or_else(|| {
            CliError::usage(
                "--domains requires a comma-separated list of exact '<domain-id>@<semver>' identities",
            )
        })?;
        if domain_id.is_empty() || domain_version.is_empty() {
            return Err(CliError::usage(
                "--domains requires a comma-separated list of exact '<domain-id>@<semver>' identities",
            ));
        }
        if domain_id == id {
            return Err(CliError::usage(
                "a Meta Harness cannot include itself in its Domain composition",
            ));
        }
        PackageDependency::new(domain_id, domain_version, false).map_err(|error| {
            CliError::usage(format!("starter Domain identity is invalid: {error}"))
        })?;
        HarnessId::new(domain_id.to_owned())
            .map_err(|error| CliError::usage(format!("starter Domain ID is invalid: {error}")))?;
        exact_domains.push(MetaHarnessStarterDomain {
            id: domain_id.to_owned(),
            version: domain_version.to_owned(),
        });
    }
    exact_domains.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.version.cmp(&right.version))
    });
    let composition = MetaComposition::new(
        exact_domains
            .iter()
            .map(|domain| HarnessId::new(domain.id.clone()).expect("validated Domain ID"))
            .collect(),
        max_handoffs,
    )
    .map_err(|error| CliError::usage(format!("starter composition is invalid: {error}")))?;
    let dependencies = exact_domains
        .iter()
        .map(|domain| {
            PackageDependency::new(domain.id.clone(), domain.version.clone(), false)
                .expect("validated exact Domain dependency")
        })
        .collect::<Vec<_>>();

    let artifact_value = MetaHarnessStarterArtifact {
        format_version: 1,
        kind: "meta_harness_profile",
        id,
        version,
        allowed_domains: exact_domains.clone(),
        max_handoffs,
    };
    let mut artifact = serde_json::to_vec_pretty(&artifact_value).map_err(|_| {
        CliError::internal("could not encode Meta Harness starter artifact", json!({}))
    })?;
    artifact.push(b'\n');

    let compatibility = PackageCompatibility::new(format!("pandora={}", env!("CARGO_PKG_VERSION")))
        .map_err(|error| CliError::internal(error.to_string(), json!({})))?;
    let manifest = PackageManifest::new_meta(
        id,
        version,
        publisher,
        hash_artifact(&artifact),
        dependencies,
        compatibility,
        license,
        TrustEvidence::unsigned(),
        composition,
    )
    .map_err(|error| CliError::usage(format!("starter manifest is invalid: {error}")))?;
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)
        .map_err(|_| CliError::internal("could not encode starter manifest", json!({})))?;
    manifest_bytes.push(b'\n');
    let readme = meta_harness_starter_readme(id, version, &exact_domains, max_handoffs);
    let architecture = meta_harness_starter_architecture();
    let files = [
        ("pandora.package.json", manifest_bytes.as_slice()),
        ("meta-harness.artifact", artifact.as_slice()),
        ("README.md", readme.as_bytes()),
        ("ARCHITECTURE.md", architecture.as_bytes()),
    ];
    write_scaffold_directory(&output, &files)?;

    let manifest_path = output.join("pandora.package.json");
    let artifact_path = output.join("meta-harness.artifact");
    Ok(success(
        "package scaffold",
        json!({
            "scaffold": {
                "format_version": 1,
                "kind": "meta_harness",
                "directory": output,
                "manifest": manifest_path,
                "artifact": artifact_path,
                "package": {
                    "id": manifest.id().as_str(),
                    "version": manifest.version(),
                    "content_hash": manifest.content_hash(),
                    "runtime_compatibility": manifest.compatibility().runtime(),
                    "dependencies": manifest.dependencies().iter().map(|dependency| json!({
                        "id": dependency.id().as_str(),
                        "version": dependency.version(),
                        "optional": dependency.optional(),
                    })).collect::<Vec<_>>(),
                    "allowed_domains": manifest.meta_composition().expect("Meta manifest composition").allowed_domains().iter().map(|domain| domain.as_str()).collect::<Vec<_>>(),
                    "max_handoffs": max_handoffs,
                },
            },
            "network_requested": false,
            "credential_accessed": false,
            "persisted_package": false,
            "runtime_authority": false,
            "next_steps": [
                ["pandora", "package", "validate", "--manifest", "pandora.package.json", "--artifact", "meta-harness.artifact"],
                ["pandora", "package", "admit", "--manifest", "pandora.package.json", "--artifact", "meta-harness.artifact"],
                ["pandora", "package", "enable", manifest.id().as_str(), manifest.version(), "--dry-run"],
                ["pandora", "package", "enable", manifest.id().as_str(), manifest.version(), "--yes"],
            ],
        }),
        format!(
            "Created a local Meta Harness starter at {} without admission or activation",
            output.display()
        ),
    ))
}

fn write_scaffold_directory(output: &Path, files: &[(&str, &[u8])]) -> Result<(), CliError> {
    fs::create_dir(output).map_err(|error| {
        CliError::configuration(
            "could not create the package starter directory; choose a new path whose parent exists",
            json!({"path": output, "error": error.to_string()}),
        )
    })?;
    let mut created = Vec::new();
    for (name, contents) in files {
        let path = output.join(name);
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) => {
                for created_path in &created {
                    let _ = fs::remove_file(created_path);
                }
                let _ = fs::remove_dir(output);
                return Err(CliError::configuration(
                    "could not write the package starter files",
                    json!({"path": path, "error": error.to_string()}),
                ));
            }
        };
        created.push(path.clone());
        if let Err(error) = file.write_all(contents).and_then(|()| file.sync_all()) {
            drop(file);
            for created_path in &created {
                let _ = fs::remove_file(created_path);
            }
            let _ = fs::remove_dir(output);
            return Err(CliError::configuration(
                "could not write the package starter files",
                json!({"path": path, "error": error.to_string()}),
            ));
        }
    }
    Ok(())
}

fn domain_harness_starter_readme(
    id: &str,
    version: &str,
    gene_id: &str,
    gene_version: &str,
) -> String {
    format!(
        "# Pandora Domain Harness starter\n\nThis directory is a deterministic, metadata-only Domain Harness profile for `{id}@{version}`. It owns the exact Gene dependency `{gene_id}@{gene_version}`. No generator step contacts a network, reads a credential, admits a package, enables a binding, or grants runtime authority.\n\n## Local lifecycle\n\nRun these commands from this directory:\n\n```text\npandora package validate --manifest pandora.package.json --artifact domain-harness.artifact\npandora package admit --manifest pandora.package.json --artifact domain-harness.artifact\npandora package enable <id> <version> --dry-run\npandora package enable <id> <version> --yes\npandora package inspect <id> <version>\npandora package disable <id> <version> --dry-run\npandora package disable <id> <version> --yes\n```\n\nTo exercise rollback, scaffold and admit a second exact version with the same ID, enable the first version, then the second, and run:\n\n```text\npandora package rollback <id> --dry-run\npandora package rollback <id> --yes\n```\n\nReplace the placeholders with the manifest values. Admission resolves required Genes exactly. Activation changes package availability only; governed effects still require Parliament policy evaluation, approval when applicable, and a one-shot ReferenceMonitor permit. See `ARCHITECTURE.md`.\n"
    )
}

fn domain_harness_starter_architecture() -> &'static str {
    "# Authority boundary\n\nThe Domain Harness package is declarative metadata. Its dependency list names the exact Genes it owns, and optional route hints only participate in Shadow Council selection. Equal best route claims fail closed and require explicit Harness selection.\n\nAdmission verifies the package identity, strict SemVer, exact artifact hash, runtime requirement, dependencies, and trust evidence. Admission and enablement do not execute the artifact, add an effect capability, approve an operation, or issue a permit.\n\nParliament remains the policy authority for a planned operation. ReferenceMonitor remains the sole issuer of scoped, one-shot effect permits. Neither component is replaceable or configurable through a Domain Harness manifest.\n"
}

fn meta_harness_starter_readme(
    id: &str,
    version: &str,
    domains: &[MetaHarnessStarterDomain],
    max_handoffs: u32,
) -> String {
    let exact_domains = domains
        .iter()
        .map(|domain| format!("{}@{}", domain.id, domain.version))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "# Pandora Meta Harness starter\n\nThis directory is a deterministic, composition-only Meta Harness profile for `{id}@{version}`. It coordinates only `{exact_domains}` with a ceiling of {max_handoffs} handoffs. Generation is local-only: it does not contact a network, read a credential, admit or enable a package, execute an effect, or grant runtime authority.\n\n## Local lifecycle\n\nRun from this directory:\n\n```text\npandora package validate --manifest pandora.package.json --artifact meta-harness.artifact\npandora package admit --manifest pandora.package.json --artifact meta-harness.artifact\npandora package enable <id> <version> --dry-run\npandora package enable <id> <version> --yes\npandora package inspect <id> <version>\npandora package disable <id> <version> --dry-run\npandora package disable <id> <version> --yes\n```\n\nTo exercise rollback, scaffold and admit a second exact version with the same ID, enable the first and then the second, and run `pandora package rollback <id> --dry-run` followed by `--yes`. Every required custom Domain must already be enabled at its exact dependency version. See `ARCHITECTURE.md`.\n"
    )
}

fn meta_harness_starter_architecture() -> &'static str {
    "# Authority boundary\n\nThis Meta Harness is declarative composition metadata. It may coordinate only the exact Domain Harness IDs listed in `meta_composition`, and every plan is rejected before execution when it names an undeclared Domain or exceeds `max_handoffs`.\n\nThe Meta Harness owns no Genes and executes no effects. It cannot add capabilities, approve an operation, issue a permit, activate itself, replace `core-source`, or change package trust. Parliament remains policy authority; ReferenceMonitor remains the sole one-shot permit issuer.\n\nAdmission validates identity, runtime compatibility, exact dependencies, composition kind, and artifact hash. Confirmed enablement changes only the exact package binding. Disable or rollback preserves the compiled fallback and grants no authority.\n"
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
    if !manifest
        .compatibility()
        .matches_runtime(env!("CARGO_PKG_VERSION"))
        .map_err(|error| CliError::usage(format!("package manifest is invalid: {error}")))?
    {
        return Err(CliError::execution(
            "package is incompatible with this Pandora runtime",
            json!({
                "required": manifest.compatibility().runtime(),
                "actual": env!("CARGO_PKG_VERSION"),
            }),
        ));
    }
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
    ensure_safe_package_input(path, "manifest")?;
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

fn trust_root(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("package trust-root requires 'add', 'list', or 'revoke'"))?;
    match subcommand.as_str() {
        "add" => trust_root_add(&args[1..]),
        "list" => trust_root_list(&args[1..]),
        "revoke" => trust_root_revoke(&args[1..]),
        _ => Err(CliError::usage(format!(
            "unknown package trust-root command '{subcommand}'"
        ))),
    }
}

fn trust_root_add(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "publisher",
            "key-id",
            "public-key",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package trust-root add does not accept positional arguments",
        ));
    }
    let publisher = parsed
        .value("publisher")
        .ok_or_else(|| CliError::usage("package trust-root add requires '--publisher <name>'"))?;
    let key_id = parsed
        .value("key-id")
        .ok_or_else(|| CliError::usage("package trust-root add requires '--key-id <id>'"))?;
    let public_key = parsed.value("public-key").ok_or_else(|| {
        CliError::usage("package trust-root add requires '--public-key <hex-or-base64>'")
    })?;
    let root = store(&parsed)?
        .add_publisher_trust_root(
            publisher,
            key_id,
            public_key,
            crate::commands::timestamp().as_unix_seconds(),
        )
        .map_err(store_error)?;
    Ok(success(
        "package trust-root add",
        trust_root_value(&root),
        format!(
            "Added publisher trust root {} for {}",
            root.key_id(),
            root.publisher()
        ),
    ))
}

fn trust_root_list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package trust-root list does not accept positional arguments",
        ));
    }
    let roots = store(&parsed)?
        .list_publisher_trust_roots()
        .map_err(store_error)?;
    let count = roots.len();
    Ok(success(
        "package trust-root list",
        json!({
            "roots": roots.iter().map(trust_root_value).collect::<Vec<_>>(),
            "count": count,
            "durability": "package-store",
        }),
        format!("Listed {count} publisher trust root(s)"),
    ))
}

fn trust_root_revoke(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "publisher",
            "key-id",
            "yes",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package trust-root revoke does not accept positional arguments",
        ));
    }
    if parsed.value("yes").is_none() {
        return Err(CliError::usage(
            "package trust-root revoke requires '--yes'",
        ));
    }
    let publisher = parsed.value("publisher").ok_or_else(|| {
        CliError::usage("package trust-root revoke requires '--publisher <name>'")
    })?;
    let key_id = parsed
        .value("key-id")
        .ok_or_else(|| CliError::usage("package trust-root revoke requires '--key-id <id>'"))?;
    let mut config = load_config(&parsed)?;
    let distribution = distribution_store(&config)?;
    let affected = distribution
        .records_for_publisher_key(publisher, key_id)
        .map_err(distribution_error)?;
    let skills_root = config.data_dir().join("skills");
    fs::create_dir_all(&skills_root)
        .map_err(|_| CliError::configuration("could not open the managed Skill root", json!({})))?;
    let skill_engine = SkillEngine::discover(&skills_root)
        .map_err(|error| CliError::execution(error.to_string(), json!({})))?;
    let mut quarantined_skills = Vec::new();
    let mut quarantined_providers = Vec::new();
    for record in affected
        .iter()
        .filter(|record| record.admitted_at().is_some())
    {
        let local_id = package_leaf_id(record.manifest().id())?;
        match record.manifest().kind() {
            PackageKind::Skill if skill_engine.inspect(local_id).is_ok() => {
                skill_engine
                    .suspend(local_id)
                    .map_err(|error| CliError::execution(error.to_string(), json!({})))?;
                quarantined_skills.push(local_id.to_owned());
            }
            PackageKind::Provider if config.quarantine_provider_profile(local_id) => {
                quarantined_providers.push(local_id.to_owned());
            }
            _ => {}
        }
    }
    if !quarantined_providers.is_empty() {
        write_config(&config)?;
    }
    let occurred_at = crate::commands::timestamp().as_unix_seconds();
    let root = PackageStore::open(config.data_dir().join("packages.sqlite3"))
        .map_err(store_error)?
        .revoke_publisher_trust_root(publisher, key_id, occurred_at)
        .map_err(store_error)?;
    let revoked_packages = distribution
        .revoke_publisher_key(publisher, key_id, occurred_at)
        .map_err(distribution_error)?;
    let mut value = trust_root_value(&root);
    value["revoked_distributions"] = json!(
        revoked_packages
            .iter()
            .map(|record| json!({
                "id": record.manifest().id().as_str(),
                "version": record.manifest().version(),
                "kind": record.manifest().kind().as_str(),
            }))
            .collect::<Vec<_>>()
    );
    value["quarantined_skills"] = json!(quarantined_skills);
    value["quarantined_providers"] = json!(quarantined_providers);
    Ok(success(
        "package trust-root revoke",
        value,
        format!(
            "Revoked publisher trust root {} for {}",
            root.key_id(),
            root.publisher()
        ),
    ))
}

fn trust_root_value(root: &pandora_runtime::PublisherTrustRootRecord) -> Value {
    json!({
        "publisher": root.publisher(),
        "key_id": root.key_id(),
        "public_key": root.public_key(),
        "added_at": root.added_at(),
        "revoked_at": root.revoked_at(),
        "active": root.active(),
    })
}

fn transparency(args: &[String]) -> Result<CommandResult, CliError> {
    let subcommand = args
        .first()
        .ok_or_else(|| CliError::usage("package transparency requires 'list' or 'inspect'"))?;
    match subcommand.as_str() {
        "list" => transparency_list(&args[1..]),
        "inspect" => transparency_inspect(&args[1..]),
        _ => Err(CliError::usage(format!(
            "unknown package transparency command '{subcommand}'"
        ))),
    }
}

fn transparency_list(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(
        args,
        &[
            "config",
            "data-dir",
            "workspace",
            "event-kind",
            "outcome",
            "limit",
        ],
    )?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package transparency list does not accept positional arguments",
        ));
    }
    let event_kind = parsed
        .value("event-kind")
        .map(parse_transparency_event_kind)
        .transpose()?;
    let outcome = parsed
        .value("outcome")
        .map(parse_transparency_outcome)
        .transpose()?;
    let limit = parsed.value("limit").map_or(Ok(50_usize), |value| {
        value.parse::<usize>().map_err(|_| {
            CliError::usage(format!(
                "--limit requires an integer between 1 and {MAX_PACKAGE_TRANSPARENCY_LIST}"
            ))
        })
    })?;
    if !(1..=MAX_PACKAGE_TRANSPARENCY_LIST).contains(&limit) {
        return Err(CliError::usage(format!(
            "--limit requires an integer between 1 and {MAX_PACKAGE_TRANSPARENCY_LIST}"
        )));
    }
    let events = store(&parsed)?
        .list_transparency_events(event_kind, outcome, limit)
        .map_err(store_error)?;
    let count = events.len();
    Ok(success(
        "package transparency list",
        json!({
            "events": events.iter().map(transparency_event_value).collect::<Vec<_>>(),
            "count": count,
            "filters": {
                "event_kind": event_kind.map(PackageTransparencyEventKind::as_str),
                "outcome": outcome.map(PackageTransparencyOutcome::as_str),
                "limit": limit,
            },
            "durability": "append-only-sqlite",
            "integrity": "sha256-event-chain",
            "runtime_authority": false,
        }),
        format!("Listed {count} package transparency event(s)"),
    ))
}

fn transparency_inspect(args: &[String]) -> Result<CommandResult, CliError> {
    let parsed = parse_options(args, &["config", "data-dir", "workspace", "sequence"])?;
    if !parsed.positionals.is_empty() {
        return Err(CliError::usage(
            "package transparency inspect does not accept positional arguments",
        ));
    }
    let sequence = parsed
        .value("sequence")
        .ok_or_else(|| {
            CliError::usage("package transparency inspect requires '--sequence <positive-id>'")
        })?
        .parse::<u64>()
        .map_err(|_| {
            CliError::usage("package transparency inspect requires a positive integer sequence")
        })?;
    if sequence == 0 {
        return Err(CliError::usage(
            "package transparency inspect requires a positive integer sequence",
        ));
    }
    let event = store(&parsed)?
        .transparency_event(sequence)
        .map_err(store_error)?
        .ok_or_else(|| {
            CliError::execution(
                "package transparency event was not found",
                json!({"sequence": sequence}),
            )
        })?;
    Ok(success(
        "package transparency inspect",
        json!({
            "event": transparency_event_value(&event),
            "durability": "append-only-sqlite",
            "integrity": "sha256-event-chain",
            "runtime_authority": false,
        }),
        format!("Inspected package transparency event {sequence}"),
    ))
}

fn parse_transparency_event_kind(value: &str) -> Result<PackageTransparencyEventKind, CliError> {
    match value {
        "trust_root_added" => Ok(PackageTransparencyEventKind::TrustRootAdded),
        "trust_root_revoked" => Ok(PackageTransparencyEventKind::TrustRootRevoked),
        "admission_decision" => Ok(PackageTransparencyEventKind::AdmissionDecision),
        _ => Err(CliError::usage(
            "--event-kind requires trust_root_added, trust_root_revoked, or admission_decision",
        )),
    }
}

fn parse_transparency_outcome(value: &str) -> Result<PackageTransparencyOutcome, CliError> {
    match value {
        "allowed" => Ok(PackageTransparencyOutcome::Allowed),
        "denied" => Ok(PackageTransparencyOutcome::Denied),
        _ => Err(CliError::usage("--outcome requires allowed or denied")),
    }
}

fn transparency_event_value(event: &PackageTransparencyEvent) -> Value {
    json!({
        "sequence": event.sequence(),
        "event_kind": event.event_kind().as_str(),
        "outcome": event.outcome().as_str(),
        "occurred_at": event.occurred_at(),
        "publisher": event.publisher(),
        "key_id": event.key_id(),
        "package_id": event.package_id(),
        "package_version": event.package_version(),
        "subject_digest": event.subject_digest(),
        "artifact_digest": event.artifact_digest(),
        "reason_code": event.reason_code(),
        "previous_event_digest": event.previous_event_digest(),
        "event_digest": event.event_digest(),
    })
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

fn command_path(
    parsed: &super::ParsedArgs,
    name: &str,
    command: &str,
) -> Result<PathBuf, CliError> {
    parsed
        .value(name)
        .map(PathBuf::from)
        .ok_or_else(|| CliError::usage(format!("{command} requires '--{name} <path>'")))
}

fn required_path(parsed: &super::ParsedArgs, name: &str) -> Result<PathBuf, CliError> {
    parsed
        .value(name)
        .map(PathBuf::from)
        .ok_or_else(|| CliError::usage(format!("package admit requires '--{name} <path>'")))
}

fn read_artifact(path: &Path) -> Result<Vec<u8>, CliError> {
    ensure_safe_package_input(path, "artifact")?;
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

fn ensure_safe_package_input(path: &Path, label: &'static str) -> Result<(), CliError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        CliError::configuration(
            format!("could not inspect package {label}"),
            json!({"path": path, "error": error.to_string()}),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CliError::configuration(
            format!("package {label} path is unsafe"),
            json!({"path": path, "reason": "symlinks and non-files are not allowed"}),
        ));
    }
    for parent in path.ancestors().skip(1) {
        if parent.as_os_str().is_empty() {
            continue;
        }
        if fs::symlink_metadata(parent).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(CliError::configuration(
                format!("package {label} path is unsafe"),
                json!({"path": path, "reason": "symlink path components are not allowed"}),
            ));
        }
    }
    Ok(())
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
        }) || (record.manifest().kind() == PackageKind::MetaHarness
            && record
                .manifest()
                .meta_composition()
                .is_some_and(|composition| {
                    composition
                        .allowed_domains()
                        .iter()
                        .any(|domain| domain.as_str() == dependency.id().as_str())
                })
            && builtin_harnesses().into_iter().any(|harness| {
                harness.manifest().id().as_str() == dependency.id().as_str()
                    && harness.manifest().version() == dependency.version()
                    && harness.manifest().kind() == pandora_types::HarnessKind::Domain
            }));
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
            if record
                .manifest()
                .dependencies()
                .iter()
                .any(|dependency| dependency.id().as_str() == domain.as_str())
            {
                continue;
            }
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
    value["owning_domains"] = if record.manifest().kind() == PackageKind::Gene {
        json!(
            store
                .list()
                .map_err(store_error)?
                .into_iter()
                .filter(|candidate| candidate.manifest().kind() == PackageKind::DomainHarness)
                .filter(|candidate| {
                    candidate
                        .manifest()
                        .dependencies()
                        .iter()
                        .any(|dependency| {
                            dependency.id() == record.manifest().id()
                                && dependency.version() == record.manifest().version()
                        })
                })
                .map(|candidate| json!({
                    "id": candidate.manifest().id().as_str(),
                    "version": candidate.manifest().version(),
                    "state": candidate.state().as_str(),
                }))
                .collect::<Vec<_>>()
        )
    } else {
        json!([])
    };
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
        "domain_routing": manifest.domain_routing().map(|routing| json!({
            "hints": routing.hints(),
            "auto_route": true,
        })),
        "gene_contract": manifest.gene_contract().map(|contract| json!({
            "execution": contract.execution().as_str(),
            "capabilities": contract.capabilities().iter().map(|capability| capability.as_str()).collect::<Vec<_>>(),
            "approval_required": contract.approval_required(),
            "direct_executor_access": false,
        })),
        "provenance": {
            "publisher": manifest.publisher(),
            "content_hash": manifest.content_hash(),
            "trust_level": manifest.trust().level(),
            "artifact_verified": true,
        },
        "replaces_builtin": replaceable_builtin_harness_kind(manifest.id().as_str())
            .is_some_and(|kind| PackageKind::from(kind) == manifest.kind()),
        "state": record.state().as_str(),
        "runtime_authority": record.grants_runtime_authority(),
    })
}

fn registry_connection(
    parsed: &super::ParsedArgs,
) -> Result<
    (
        pandora_runtime::config::RuntimeConfig,
        String,
        Option<String>,
        Option<String>,
    ),
    CliError,
> {
    if parsed.value("registry").is_some() && parsed.value("registry-profile").is_some() {
        return Err(CliError::usage(
            "package registry access accepts either '--registry' or '--registry-profile', not both",
        ));
    }
    let config = load_config(parsed)?;
    let selected_profile = parsed.value("registry-profile").or_else(|| {
        parsed
            .value("registry")
            .is_none()
            .then(|| config.active_registry())
            .flatten()
    });
    let (registry, profile_token_env, profile_name) = if let Some(name) = selected_profile {
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
                    "package registry access requires a profile, '--registry <url>', or PANDORA_REGISTRY_URL",
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
    Ok((config, registry, profile_name, token))
}

fn distribution_store(
    config: &pandora_runtime::config::RuntimeConfig,
) -> Result<PackageDistributionStore, CliError> {
    PackageDistributionStore::open(config.data_dir().join("packages.sqlite3"))
        .map_err(distribution_error)
}

fn distribution_identity(
    parsed: &super::ParsedArgs,
    command: &str,
) -> Result<(PackageId, String), CliError> {
    if parsed.positionals.len() != 2 {
        return Err(CliError::usage(format!(
            "{command} requires an ID and exact version"
        )));
    }
    let id = PackageId::new(parsed.positionals[0].clone())
        .map_err(|_| CliError::usage("package ID is invalid"))?;
    let version = parsed.positionals[1].clone();
    Version::parse(&version)
        .map_err(|_| CliError::usage("package version must be exact SemVer"))?;
    Ok((id, version))
}

fn distribution_limit(value: Option<&str>) -> Result<usize, CliError> {
    let limit = value
        .unwrap_or("64")
        .parse::<usize>()
        .map_err(|_| CliError::usage("--limit must be a positive integer"))?;
    if !(1..=MAX_DISTRIBUTION_LIST).contains(&limit) {
        return Err(CliError::usage(format!(
            "--limit must be between 1 and {MAX_DISTRIBUTION_LIST}"
        )));
    }
    Ok(limit)
}

fn distribution_record_value(
    store: &PackageDistributionStore,
    record: &DistributionRecord,
) -> Result<Value, CliError> {
    let manifest = record.manifest();
    let verification = match store.verify_current(manifest.id(), manifest.version()) {
        Ok(_) => "verified",
        Err(PackageDistributionError::Revoked | PackageDistributionError::RevokedPublisher) => {
            "revoked"
        }
        Err(_) => "failed_closed",
    };
    let binding = store.binding(manifest.id()).map_err(distribution_error)?;
    Ok(json!({
        "id": manifest.id().as_str(),
        "version": manifest.version(),
        "kind": manifest.kind().as_str(),
        "publisher": manifest.publisher(),
        "publisher_key_id": record.publisher_key_id(),
        "content_hash": manifest.content_hash(),
        "manifest_digest": record.manifest_digest().map_err(distribution_error)?,
        "artifact_digest": record.artifact_digest(),
        "dependencies": manifest.dependencies().iter().map(|dependency| json!({
            "id": dependency.id().as_str(),
            "version": dependency.version(),
            "optional": dependency.optional(),
        })).collect::<Vec<_>>(),
        "compatibility": manifest.compatibility().runtime(),
        "license": manifest.license(),
        "trust": {
            "declared_level": manifest.trust().level(),
            "verification": verification,
            "signature_present": manifest.trust().signature().is_some(),
            "publisher_key_present": manifest.trust().public_key().is_some(),
        },
        "source": {
            "kind": record.source().kind().as_str(),
            "locator": record.source().locator(),
            "revision": record.source().revision(),
        },
        "cached_at": record.cached_at(),
        "admitted_at": record.admitted_at(),
        "state": record.state().as_str(),
        "admission": {
            "active_version": binding.as_ref().map(|binding| binding.active_version()),
            "previous_version": binding.as_ref().and_then(|binding| binding.previous_version()),
            "generation": binding.as_ref().map_or(0, |binding| binding.generation()),
            "this_version_admitted": binding.as_ref().is_some_and(|binding| binding.active_version() == manifest.version()),
        },
        "download_authority": "cache_only",
        "runtime_authority": false,
    }))
}

fn distribution_event_value(event: &DistributionEvent) -> Value {
    json!({
        "sequence": event.sequence(),
        "event_kind": event.event_kind().as_str(),
        "occurred_at": event.occurred_at(),
        "package_id": event.package_id(),
        "package_version": event.package_version(),
        "package_kind": event.package_kind().as_str(),
        "publisher": event.publisher(),
        "publisher_key_id": event.publisher_key_id(),
        "manifest_digest": event.manifest_digest(),
        "artifact_digest": event.artifact_digest(),
        "source": {
            "kind": event.source_kind().as_str(),
            "locator": event.source_locator(),
            "revision": event.source_revision(),
        },
        "previous_event_digest": event.previous_event_digest(),
        "event_digest": event.event_digest(),
    })
}

fn package_leaf_id(id: &PackageId) -> Result<&str, CliError> {
    id.as_str()
        .rsplit('/')
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| CliError::execution("package ID has no local boundary identity", json!({})))
}

const fn package_admission_boundary(kind: PackageKind) -> &'static str {
    match kind {
        PackageKind::Gene | PackageKind::DomainHarness | PackageKind::MetaHarness => {
            "harness_registry"
        }
        PackageKind::Skill => "skill_engine_disabled",
        PackageKind::Provider => "provider_catalog_inactive",
        PackageKind::SourceHarness => "constitutional_source",
        PackageKind::Package => "data_only",
    }
}

fn distribution_error(error: PackageDistributionError) -> CliError {
    CliError::execution(error.to_string(), json!({}))
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
